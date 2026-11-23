//! Reads Qdrant's JSON replies into the neutral vector types.

use dbclient_core::vector::{
    CollectionInfo, PayloadField, Point, PointId, PointVector, VectorData, VectorKind, VectorSpec,
};
use dbclient_core::{Error, Result};
use serde_json::{Map, Value, json};

/// A reply that is not shaped like the API's. The start of it is kept: it
/// usually shows what answered instead (a proxy's error page, another service).
pub fn unexpected(text: &str) -> Error {
    let start: String = text.trim().chars().take(200).collect();
    Error::Internal(format!("unexpected reply from the server: {start}"))
}

pub fn id(value: &Value) -> Result<PointId> {
    match value {
        Value::Number(number) => number.as_u64().map(PointId::Number),
        Value::String(uuid) => Some(PointId::Uuid(uuid.clone())),
        _ => None,
    }
    .ok_or_else(|| unexpected(&value.to_string()))
}

/// The id as the API takes it: a JSON number or a UUID string.
pub fn id_to_json(id: &PointId) -> Value {
    match id {
        PointId::Number(number) => json!(number),
        PointId::Uuid(uuid) => json!(uuid),
    }
}

/// A point of a search, scroll or lookup reply. They share the fields read here.
pub fn point(value: &Value) -> Result<Point> {
    Ok(Point {
        id: id(&value["id"])?,
        payload: value.get("payload").filter(|p| !p.is_null()).cloned(),
        vectors: vectors(value.get("vector")),
    })
}

/// The `vector` field: one unnamed vector, or an object of named ones.
fn vectors(value: Option<&Value>) -> Vec<PointVector> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(named)) => named
            .iter()
            .map(|(name, value)| PointVector {
                name: Some(name.clone()),
                data: vector_data(value),
            })
            .collect(),
        Some(unnamed) => vec![PointVector {
            name: None,
            data: vector_data(unnamed),
        }],
    }
}

fn vector_data(value: &Value) -> VectorData {
    let read = || -> Option<VectorData> {
        match value {
            Value::Array(items) if items.iter().all(Value::is_array) && !items.is_empty() => {
                let vectors = items.iter().map(numbers).collect::<Option<_>>()?;
                Some(VectorData::Multi { vectors })
            }
            Value::Array(_) => Some(VectorData::Dense {
                values: numbers(value)?,
            }),
            Value::Object(sparse) => Some(VectorData::Sparse {
                indices: sparse
                    .get("indices")?
                    .as_array()?
                    .iter()
                    .map(|i| i.as_u64().and_then(|i| u32::try_from(i).ok()))
                    .collect::<Option<_>>()?,
                values: numbers(sparse.get("values")?)?,
            }),
            _ => None,
        }
    };
    read().unwrap_or_else(|| VectorData::Other { raw: value.clone() })
}

fn numbers(value: &Value) -> Option<Vec<f32>> {
    value
        .as_array()?
        .iter()
        .map(|n| n.as_f64().map(|n| n as f32))
        .collect()
}

pub fn collection_info(name: &str, result: &Value) -> CollectionInfo {
    let params = &result["config"]["params"];
    let mut vectors = Vec::new();
    match &params["vectors"] {
        // A plain collection describes its one vector directly. A named
        // vector could be called "size" too, but then it holds an object.
        Value::Object(unnamed) if unnamed.get("size").is_some_and(Value::is_number) => {
            vectors.push(vector_spec(None, unnamed));
        }
        Value::Object(named) => {
            for (name, spec) in named {
                if let Value::Object(spec) = spec {
                    vectors.push(vector_spec(Some(name.clone()), spec));
                }
            }
        }
        _ => {}
    }
    if let Value::Object(sparse) = &params["sparse_vectors"] {
        for name in sparse.keys() {
            vectors.push(VectorSpec {
                name: Some(name.clone()),
                kind: VectorKind::Sparse,
                size: None,
                distance: None,
                // Sparse vectors are compared by dot product.
                higher_is_better: true,
            });
        }
    }

    let payload_fields = match &result["payload_schema"] {
        Value::Object(fields) => fields
            .iter()
            .map(|(name, field)| PayloadField {
                name: name.clone(),
                kind: field["data_type"].as_str().unwrap_or_default().to_owned(),
                points: field["points"].as_u64(),
            })
            .collect(),
        _ => Vec::new(),
    };

    CollectionInfo {
        name: name.to_owned(),
        status: result["status"].as_str().map(str::to_owned),
        points: result["points_count"].as_u64(),
        vectors,
        payload_fields,
    }
}

fn vector_spec(name: Option<String>, spec: &Map<String, Value>) -> VectorSpec {
    let distance = spec
        .get("distance")
        .and_then(Value::as_str)
        .map(str::to_owned);
    VectorSpec {
        name,
        kind: if spec.get("multivector_config").is_some_and(Value::is_object) {
            VectorKind::Multi
        } else {
            VectorKind::Dense
        },
        size: spec.get("size").and_then(Value::as_u64),
        // The two metrics that are distances; every other one is a similarity.
        higher_is_better: !matches!(distance.as_deref(), Some("Euclid" | "Manhattan")),
        distance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points() {
        let read = point(&json!({
            "id": 18446744073709551615u64,
            "version": 3,
            "score": 0.5,
            "payload": { "city": "Berlin" },
            "vector": [1.0, 0.5]
        }))
        .unwrap();
        assert_eq!(read.id, PointId::Number(u64::MAX));
        assert_eq!(read.payload, Some(json!({ "city": "Berlin" })));
        assert_eq!(
            read.vectors,
            vec![PointVector {
                name: None,
                data: VectorData::Dense {
                    values: vec![1.0, 0.5]
                }
            }]
        );

        // Without payload and vector the keys are absent.
        let bare = point(&json!({ "id": "6f1e7f0a-0000-4000-8000-000000000001" })).unwrap();
        assert_eq!(
            bare.id,
            PointId::Uuid("6f1e7f0a-0000-4000-8000-000000000001".into())
        );
        assert_eq!((bare.payload, bare.vectors), (None, Vec::new()));

        assert!(point(&json!({ "id": -1 })).is_err());
    }

    #[test]
    fn every_vector_layout() {
        let read = point(&json!({
            "id": 1,
            "vector": {
                "text": [1.0, 0.0],
                "tokens": [[1.0, 0.0], [0.0, 1.0]],
                "words": { "indices": [1, 7], "values": [0.5, 0.25] },
                "later": "something new"
            }
        }))
        .unwrap();
        let by_name = |name: &str| {
            let found = read
                .vectors
                .iter()
                .find(|v| v.name.as_deref() == Some(name));
            found.unwrap().data.clone()
        };
        assert_eq!(
            by_name("text"),
            VectorData::Dense {
                values: vec![1.0, 0.0]
            }
        );
        assert_eq!(
            by_name("tokens"),
            VectorData::Multi {
                vectors: vec![vec![1.0, 0.0], vec![0.0, 1.0]]
            }
        );
        assert_eq!(
            by_name("words"),
            VectorData::Sparse {
                indices: vec![1, 7],
                values: vec![0.5, 0.25]
            }
        );
        assert_eq!(
            by_name("later"),
            VectorData::Other {
                raw: json!("something new")
            }
        );
    }

    #[test]
    fn collection_with_one_vector() {
        let info = collection_info(
            "articles",
            &json!({
                "status": "green",
                "points_count": 12400,
                "config": { "params": { "vectors": { "size": 1536, "distance": "Cosine" } } },
                "payload_schema": { "lang": { "data_type": "keyword", "points": 12000 } }
            }),
        );
        assert_eq!(info.points, Some(12400));
        assert_eq!(
            info.vectors,
            vec![VectorSpec {
                name: None,
                kind: VectorKind::Dense,
                size: Some(1536),
                distance: Some("Cosine".into()),
                higher_is_better: true,
            }]
        );
        assert_eq!(
            info.payload_fields,
            vec![PayloadField {
                name: "lang".into(),
                kind: "keyword".into(),
                points: Some(12000)
            }]
        );
    }

    #[test]
    fn collection_with_named_vectors() {
        let info = collection_info(
            "mixed",
            &json!({
                "config": { "params": {
                    "vectors": {
                        "size": { "size": 2, "distance": "Euclid" },
                        "tokens": {
                            "size": 4, "distance": "Dot",
                            "multivector_config": { "comparator": "max_sim" }
                        }
                    },
                    "sparse_vectors": { "words": {} }
                } }
            }),
        );
        let summary: Vec<_> = info
            .vectors
            .iter()
            .map(|v| (v.name.as_deref(), v.kind, v.size, v.higher_is_better))
            .collect();
        assert_eq!(
            summary,
            vec![
                (Some("size"), VectorKind::Dense, Some(2), false),
                (Some("tokens"), VectorKind::Multi, Some(4), true),
                (Some("words"), VectorKind::Sparse, None, true),
            ]
        );
        assert_eq!((info.status, info.points), (None, None));
    }
}
