//! pgvector: vector search in PostgreSQL.
//!
//! The connection is a PostgreSQL connection (driver-postgres), so query tabs
//! run SQL as usual. On top of it, tables with a `vector` column are shown as
//! the collections of the vector view:
//!
//! - a point is a row; its id is the table's single-column primary key;
//! - each `vector` column is a named vector;
//! - the payload is the row's other columns, as `to_jsonb` writes them.
//!
//! pgvector stores no metric on a column. The metric is taken from the
//! operator class of the column's index (`vector_cosine_ops` and so on), else
//! from the `metric` driver option, else cosine.
//!
//! Everything here reads. Collections are named by table, with the schema in
//! front when it is not the connection's current schema (`rag.chunks`).

mod sql;

use std::collections::BTreeMap;
use std::time::Instant;

use async_trait::async_trait;
use dbclient_core::vector::{
    CollectionInfo, PayloadField, Point, PointId, PointPage, PointVector, ScoredPoint,
    ScrollRequest, SearchQuery, SearchRequest, SearchResult, VectorData, VectorKind, VectorSpec,
    VectorStore,
};
use dbclient_core::{
    Capabilities, Connection, ConnectionConfig, DataModel, DbObject, Driver, DriverInfo, Error,
    ForeignKey, IndexDescription, Namespace, ObjectDescription, ObjectKind, QueryRequest,
    QueryResult, Reference, Result, TriggerDescription, Value,
};
use dbclient_driver_postgres::PostgresDriver;
use sql::{Metric, literal};

pub struct PgvectorDriver;

#[async_trait]
impl Driver for PgvectorDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "pgvector",
            name: "pgvector",
            model: DataModel::Vector,
            capabilities: Capabilities {
                vector_search: true,
                ..PostgresDriver.info().capabilities
            },
            default_port: Some(5432),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let metric = match config.params.get("metric").filter(|m| !m.trim().is_empty()) {
            None => None,
            Some(text) => Some(Metric::parse(text).ok_or_else(|| {
                Error::InvalidConfig(format!("metric {text:?}: use cosine, l2, ip or l1"))
            })?),
        };
        let inner = PostgresDriver.connect(config).await?;
        Ok(Box::new(PgvectorConnection { inner, metric }))
    }
}

struct PgvectorConnection {
    inner: Box<dyn Connection>,
    /// The `metric` driver option, used where no index names one.
    metric: Option<Metric>,
}

/// A `vector` column of a table.
struct VectorColumn {
    name: String,
    size: Option<u64>,
    metric: Metric,
}

/// What the vector operations need to know about a table.
struct Table {
    /// Quoted and schema-qualified, ready for SQL.
    qualified: String,
    /// The primary key column, quoted.
    key: String,
    /// Column names left out of the payload: the key and the vectors.
    hidden: Vec<String>,
    vectors: Vec<VectorColumn>,
    payload_fields: Vec<PayloadField>,
    estimate: Option<i64>,
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Text(s) | Value::Decimal(s) | Value::Temporal(s) => Some(s.clone()),
        Value::Int(n) => Some(n.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Json(j) => Some(j.to_string()),
        Value::Bytes(_) => None,
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Float(f) => Some(*f),
        Value::Int(n) => Some(*n as f64),
        Value::Decimal(s) | Value::Text(s) => s.parse().ok(),
        _ => None,
    }
}

impl PgvectorConnection {
    async fn rows(&self, statement: String) -> Result<Vec<Vec<Value>>> {
        Ok(self
            .inner
            .execute(QueryRequest {
                statement,
                max_rows: None,
            })
            .await?
            .rows)
    }

    async fn current_schema(&self) -> Result<String> {
        Ok(self
            .inner
            .current_namespace()
            .await?
            .unwrap_or_else(|| "public".into()))
    }

    /// Every table with a vector column: (schema, table, estimated rows), sorted.
    async fn vector_tables(&self) -> Result<Vec<(String, String, Option<i64>)>> {
        let rows = self
            .rows(
                "SELECT DISTINCT n.nspname, c.relname, c.reltuples::bigint \
                 FROM pg_attribute a \
                 JOIN pg_class c ON c.oid = a.attrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 JOIN pg_type t ON t.oid = a.atttypid \
                 WHERE t.typname = 'vector' AND a.attnum > 0 AND NOT a.attisdropped \
                   AND c.relkind IN ('r', 'p') \
                   AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
                 ORDER BY 1, 2"
                    .into(),
            )
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some((
                    text(&r[0])?,
                    text(&r[1])?,
                    // -1 means never analyzed: no estimate.
                    number(&r[2]).map(|n| n as i64).filter(|n| *n >= 0),
                ))
            })
            .collect())
    }

    fn collection_name(current: &str, schema: &str, table: &str) -> String {
        if schema == current {
            table.to_owned()
        } else {
            format!("{schema}.{table}")
        }
    }

    /// The schema and table a collection name stands for.
    async fn resolve(&self, collection: &str) -> Result<(String, String, Option<i64>)> {
        let current = self.current_schema().await?;
        self.vector_tables()
            .await?
            .into_iter()
            .find(|(schema, table, _)| Self::collection_name(&current, schema, table) == collection)
            .ok_or_else(|| {
                Error::Query(format!("no table with a vector column named {collection}"))
            })
    }

    async fn table(&self, collection: &str) -> Result<Table> {
        let (schema, name, estimate) = self.resolve(collection).await?;
        let qualified = format!(
            "{}.{}",
            self.inner.quote_ident(&schema),
            self.inner.quote_ident(&name)
        );
        let oid = format!("{}::regclass", literal(&qualified));

        let columns = self
            .rows(format!(
                "SELECT a.attname, t.typname, format_type(a.atttypid, a.atttypmod), a.atttypmod, \
                        COALESCE(a.attnum = ANY(i.indkey), false) \
                 FROM pg_attribute a \
                 JOIN pg_type t ON t.oid = a.atttypid \
                 LEFT JOIN pg_index i ON i.indrelid = a.attrelid AND i.indisprimary \
                 WHERE a.attrelid = {oid} AND a.attnum > 0 AND NOT a.attisdropped \
                 ORDER BY a.attnum"
            ))
            .await?;
        let metrics = self
            .rows(format!(
                "SELECT a.attname, opc.opcname \
                 FROM pg_index i \
                 CROSS JOIN LATERAL unnest(i.indkey::int2[], i.indclass::oid[]) AS k(attnum, opclass) \
                 JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum \
                 JOIN pg_opclass opc ON opc.oid = k.opclass \
                 WHERE i.indrelid = {oid} AND opc.opcname LIKE 'vector%'"
            ))
            .await?;
        let indexed: BTreeMap<String, Metric> = metrics
            .iter()
            .filter_map(|r| Some((text(&r[0])?, Metric::parse(&text(&r[1])?)?)))
            .collect();

        let mut keys = Vec::new();
        let mut vectors = Vec::new();
        let mut payload_fields = Vec::new();
        for row in &columns {
            let (Some(column), Some(base)) = (text(&row[0]), text(&row[1])) else {
                continue;
            };
            let is_key = matches!(row[4], Value::Bool(true));
            if is_key {
                keys.push(column.clone());
            }
            if base == "vector" {
                vectors.push(VectorColumn {
                    size: number(&row[3]).filter(|n| *n > 0.0).map(|n| n as u64),
                    metric: indexed
                        .get(&column)
                        .copied()
                        .or(self.metric)
                        .unwrap_or(Metric::Cosine),
                    name: column,
                });
            } else if !is_key {
                payload_fields.push(PayloadField {
                    name: column,
                    kind: text(&row[2]).unwrap_or(base),
                    points: None,
                });
            }
        }
        let [key] = keys.as_slice() else {
            return Err(Error::Unsupported(
                "pgvector tables need a primary key of one column to identify points",
            ));
        };
        let mut hidden = vec![key.clone()];
        hidden.extend(vectors.iter().map(|v| v.name.clone()));
        Ok(Table {
            key: self.inner.quote_ident(key),
            qualified,
            hidden,
            vectors,
            payload_fields,
            estimate,
        })
    }

    /// The SELECT list shared by search, scroll and get: id, payload, vectors.
    fn select_list(&self, table: &Table, with_payload: bool, with_vector: bool) -> String {
        let mut list = vec![format!("t.{}::text", table.key)];
        if with_payload {
            let hidden: Vec<String> = table.hidden.iter().map(|h| literal(h)).collect();
            list.push(format!(
                "to_jsonb(t) - ARRAY[{}]::text[]",
                hidden.join(", ")
            ));
        }
        if with_vector {
            for v in &table.vectors {
                list.push(format!("t.{}::text", self.inner.quote_ident(&v.name)));
            }
        }
        list.join(", ")
    }

    /// Reads a row produced by [`select_list`], starting at `at`.
    fn point(
        table: &Table,
        row: &[Value],
        at: usize,
        with_payload: bool,
        with_vector: bool,
    ) -> Point {
        let id = PointId::parse(&text(&row[at]).unwrap_or_default());
        let mut next = at + 1;
        let payload = with_payload.then(|| {
            next += 1;
            match &row[next - 1] {
                Value::Json(j) => j.clone(),
                other => serde_json::Value::String(text(other).unwrap_or_default()),
            }
        });
        let vectors = if with_vector {
            table
                .vectors
                .iter()
                .enumerate()
                .filter_map(|(i, v)| {
                    let raw = text(&row[next + i])?;
                    Some(PointVector {
                        name: Some(v.name.clone()),
                        data: match sql::parse_vector(&raw) {
                            Some(values) => VectorData::Dense { values },
                            None => VectorData::Other {
                                raw: serde_json::Value::String(raw),
                            },
                        },
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        Point {
            id,
            payload,
            vectors,
        }
    }
}

#[async_trait]
impl VectorStore for PgvectorConnection {
    async fn list_collections(&self) -> Result<Vec<String>> {
        let current = self.current_schema().await?;
        let mut names: Vec<String> = self
            .vector_tables()
            .await?
            .iter()
            .map(|(schema, table, _)| Self::collection_name(&current, schema, table))
            .collect();
        names.sort();
        Ok(names)
    }

    async fn collection_info(&self, collection: &str) -> Result<CollectionInfo> {
        let table = self.table(collection).await?;
        // A table that was never analyzed has no estimate; small ones are then counted.
        let points = match table.estimate {
            Some(n) => Some(n as u64),
            None => self
                .rows(format!("SELECT count(*) FROM {}", table.qualified))
                .await?
                .first()
                .and_then(|r| number(&r[0]))
                .map(|n| n as u64),
        };
        Ok(CollectionInfo {
            name: collection.to_owned(),
            status: None,
            points,
            vectors: table
                .vectors
                .iter()
                .map(|v| VectorSpec {
                    name: Some(v.name.clone()),
                    kind: VectorKind::Dense,
                    size: v.size,
                    distance: Some(v.metric.name().to_owned()),
                    higher_is_better: v.metric.higher_is_better(),
                })
                .collect(),
            payload_fields: table.payload_fields,
        })
    }

    async fn search(&self, request: SearchRequest) -> Result<SearchResult> {
        let started = Instant::now();
        let table = self.table(&request.collection).await?;
        let column = match &request.using {
            Some(name) => table.vectors.iter().find(|v| &v.name == name),
            None => table.vectors.first(),
        }
        .ok_or_else(|| Error::Query("the table has no such vector column".into()))?;
        let quoted = self.inner.quote_ident(&column.name);

        let mut conditions = vec![sql::filter("t", &request.filter).map_err(Error::Query)?];
        let query = match &request.query {
            SearchQuery::Vector { values } => {
                if let Some(size) = column.size
                    && values.len() as u64 != size
                {
                    return Err(Error::Query(format!(
                        "{} has {size} dimensions, the query vector {}",
                        column.name,
                        values.len()
                    )));
                }
                sql::vector_literal(values)
            }
            SearchQuery::Point { id } => {
                let id = literal(&id.to_string());
                let found = self
                    .rows(format!(
                        "SELECT 1 FROM {} WHERE {} = {id} AND {quoted} IS NOT NULL",
                        table.qualified, table.key
                    ))
                    .await?;
                if found.is_empty() {
                    return Err(Error::Query(format!(
                        "point {id} has no {} vector",
                        column.name
                    )));
                }
                // "Find similar" leaves the point itself out.
                conditions.push(format!("t.{} <> {id}", table.key));
                format!(
                    "(SELECT {quoted} FROM {} WHERE {} = {id})",
                    table.qualified, table.key
                )
            }
        };
        let distance = format!("t.{quoted} {} {query}", column.metric.operator());
        let statement = format!(
            "SELECT ({})::float8, {} FROM {} t WHERE t.{quoted} IS NOT NULL AND {} ORDER BY {distance} LIMIT {}",
            column.metric.score(&distance),
            self.select_list(&table, request.with_payload, request.with_vector),
            table.qualified,
            conditions.join(" AND "),
            request.limit.max(1)
        );
        let rows = self.rows(statement).await?;

        let better = column.metric.higher_is_better();
        let mut below = 0;
        let mut hits = Vec::new();
        for row in &rows {
            let score = number(&row[0]).unwrap_or(f64::NAN);
            if let Some(min) = request.min_score
                && (if better { score < min } else { score > min })
            {
                below += 1;
                continue;
            }
            hits.push(ScoredPoint {
                rank: hits.len() as u32 + 1,
                score,
                point: Self::point(&table, row, 1, request.with_payload, request.with_vector),
            });
        }
        Ok(SearchResult {
            hits,
            below_min_score: below,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn get_point(&self, collection: &str, id: &PointId) -> Result<Point> {
        let table = self.table(collection).await?;
        let rows = self
            .rows(format!(
                "SELECT {} FROM {} t WHERE t.{} = {}",
                self.select_list(&table, true, true),
                table.qualified,
                table.key,
                literal(&id.to_string())
            ))
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| Error::Query(format!("no point {id} in {collection}")))?;
        Ok(Self::point(&table, row, 0, true, true))
    }

    async fn scroll(&self, request: ScrollRequest) -> Result<PointPage> {
        let started = Instant::now();
        let table = self.table(&request.collection).await?;
        let mut conditions = vec![sql::filter("t", &request.filter).map_err(Error::Query)?];
        if let Some(cursor) = &request.cursor {
            conditions.push(format!(
                "t.{} > {}",
                table.key,
                literal(&cursor.to_string())
            ));
        }
        let limit = request.limit.max(1) as usize;
        let rows = self
            .rows(format!(
                "SELECT {} FROM {} t WHERE {} ORDER BY t.{} LIMIT {}",
                self.select_list(&table, request.with_payload, request.with_vector),
                table.qualified,
                conditions.join(" AND "),
                table.key,
                limit + 1
            ))
            .await?;
        let mut points: Vec<Point> = rows
            .iter()
            .map(|r| Self::point(&table, r, 0, request.with_payload, request.with_vector))
            .collect();
        // One row more than asked says whether there is a next page.
        let next = (points.len() > limit).then(|| {
            points.truncate(limit);
            points[limit - 1].id.clone()
        });
        Ok(PointPage {
            points,
            next,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[async_trait]
impl Connection for PgvectorConnection {
    async fn ping(&self) -> Result<()> {
        self.inner.ping().await
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        self.inner.execute_all(request).await
    }

    /// The extension's version first, as the status bar shows the first word:
    /// `0.8.1 (PostgreSQL 17.8)`. Without the extension, PostgreSQL's own.
    async fn server_version(&self) -> Result<Option<String>> {
        let postgres = self.inner.server_version().await?;
        let extension = self
            .rows("SELECT extversion FROM pg_extension WHERE extname = 'vector'".into())
            .await?
            .first()
            .and_then(|r| text(&r[0]));
        Ok(match (extension, postgres) {
            (Some(ext), Some(pg)) => Some(format!("{ext} (PostgreSQL {pg})")),
            (Some(ext), None) => Some(ext),
            (None, pg) => pg,
        })
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        self.inner.current_namespace().await
    }

    /// The schemas that hold a table with a vector column.
    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        let mut schemas: Vec<String> = self
            .vector_tables()
            .await?
            .into_iter()
            .map(|(schema, _, _)| schema)
            .collect();
        schemas.dedup();
        if schemas.is_empty() {
            schemas.push(self.current_schema().await?);
        }
        Ok(schemas.into_iter().map(|name| Namespace { name }).collect())
    }

    /// The vector tables of a schema, named as the vector view names collections.
    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let current = self.current_schema().await?;
        let wanted = namespace.unwrap_or(&current).to_owned();
        Ok(self
            .vector_tables()
            .await?
            .into_iter()
            .filter(|(schema, _, _)| *schema == wanted)
            .map(|(schema, table, estimate)| DbObject {
                name: Self::collection_name(&current, &schema, &table),
                kind: ObjectKind::Collection,
                row_estimate: estimate.map(|n| n as u64),
            })
            .collect())
    }

    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        self.inner.describe_object(namespace, name).await
    }

    async fn list_indexes(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexDescription>> {
        self.inner.list_indexes(namespace, table).await
    }

    async fn list_foreign_keys(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        self.inner.list_foreign_keys(namespace, table).await
    }

    async fn list_referencing(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<Reference>> {
        self.inner.list_referencing(namespace, table).await
    }

    async fn list_triggers(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<TriggerDescription>> {
        self.inner.list_triggers(namespace, table).await
    }

    async fn object_ddl(&self, namespace: Option<&str>, name: &str) -> Result<Option<String>> {
        self.inner.object_ddl(namespace, name).await
    }

    fn vector(&self) -> Option<&dyn VectorStore> {
        Some(self)
    }

    fn quote_ident(&self, name: &str) -> String {
        self.inner.quote_ident(name)
    }

    async fn is_alive(&self) -> bool {
        self.inner.is_alive().await
    }

    fn dialect(&self) -> dbclient_core::SqlDialect {
        self.inner.dialect()
    }

    fn transactional_ddl(&self) -> bool {
        self.inner.transactional_ddl()
    }

    async fn in_transaction(&self) -> Result<bool> {
        self.inner.in_transaction().await
    }

    async fn apply_changes(
        &self,
        changes: &dbclient_core::ChangeSet,
    ) -> Result<dbclient_core::ApplyResult> {
        self.inner.apply_changes(changes).await
    }

    async fn cancel(&self) -> Result<()> {
        self.inner.cancel().await
    }

    async fn close(&self) -> Result<()> {
        self.inner.close().await
    }
}
