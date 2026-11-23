import { msg } from "../i18n";
// The database types shown in the type picker, grouped by data model. A type
// is available when it names a driver that this build registers; the rest are
// listed as "coming soon".

export interface DatabaseType {
  /** Two-letter code shown in badges. */
  code: string;
  name: string;
  /** Id of the driver that serves this type, when one exists. */
  driver?: string;
  /** Default port, when it differs from the driver's own. */
  port?: number;
}

export const TYPE_GROUPS: { model: string; types: DatabaseType[] }[] = [
  {
    model: msg("Relational"),
    types: [
      { code: "Pg", name: "PostgreSQL", driver: "postgres" },
      { code: "My", name: "MySQL", driver: "mysql" },
      // Speaks the MySQL protocol; the MySQL driver's tests also run against it.
      { code: "Ma", name: "MariaDB", driver: "mysql" },
      { code: "Ms", name: "SQL Server", driver: "sqlserver" },
      { code: "Or", name: "Oracle" },
      { code: "Sq", name: "SQLite", driver: "sqlite" },
    ],
  },
  {
    model: msg("Distributed SQL"),
    // Speaks the PostgreSQL protocol; the PostgreSQL driver's tests also run against it.
    types: [{ code: "Cr", name: "CockroachDB", driver: "postgres", port: 26257 }, { code: "Ti", name: "TiDB" }, { code: "Yb", name: "YugabyteDB" }],
  },
  { model: msg("Analytic"), types: [{ code: "Ch", name: "ClickHouse", driver: "clickhouse" }, { code: "Dk", name: "DuckDB", driver: "duckdb" }, { code: "Sf", name: "Snowflake" }, { code: "Bq", name: "BigQuery" }] },
  { model: msg("Document"), types: [{ code: "Mg", name: "MongoDB", driver: "mongodb" }, { code: "Cb", name: "Couchbase" }] },
  { model: msg("Key-value"), types: [{ code: "Rd", name: "Redis", driver: "redis" }, { code: "Vk", name: "Valkey", driver: "redis" }] },
  { model: msg("Vector"), types: [{ code: "Qd", name: "Qdrant", driver: "qdrant" }, { code: "Pv", name: "pgvector" }, { code: "Mv", name: "Milvus" }] },
  { model: msg("Graph"), types: [{ code: "N4", name: "Neo4j", driver: "neo4j" }] },
  { model: msg("Time series"), types: [{ code: "Ts", name: "TimescaleDB" }, { code: "If", name: "InfluxDB" }] },
  { model: msg("Search"), types: [{ code: "Es", name: "Elasticsearch" }, { code: "Os", name: "OpenSearch" }] },
];

const ALL = TYPE_GROUPS.flatMap((g) => g.types);

/** The first type listed for a driver: what a connection saved without a type code is shown as. */
export const typeOfDriver = (driverId: string): DatabaseType =>
  ALL.find((t) => t.driver === driverId) ?? { code: driverId.slice(0, 2), name: driverId, driver: driverId };

/** The type with this code, falling back to the driver's own. */
export const typeOf = (code: string | null | undefined, driverId: string): DatabaseType =>
  ALL.find((t) => t.code === code && t.driver === driverId) ?? typeOfDriver(driverId);
