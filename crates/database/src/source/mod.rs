use super::*;
use sqlx::Row;
#[derive(Clone, Default)]
pub struct MacParakeetCatalog;
#[async_trait::async_trait]
impl LegacyCatalog for MacParakeetCatalog {
    async fn read(&self, path: &Path) -> Result<Vec<LegacySourceRecord>, CapabilityError> {
        let pool = raw_read_only(path).await.map_err(CapabilityError::new)?;
        // Source compatibility exception: our build schema does not contain third-party tables.
        // Source schema and rows share one read snapshot, including live WAL state.
        let mut tx = pool.begin().await.map_err(CapabilityError::new)?;
        let columns: Vec<String> = sqlx::query("PRAGMA table_info(transcriptions)")
            .fetch_all(&mut *tx)
            .await
            .map_err(CapabilityError::new)?
            .iter()
            .filter_map(|r| r.try_get::<String, _>("name").ok())
            .collect();
        if !["id", "fileName", "createdAt"]
            .iter()
            .all(|c| columns.iter().any(|x| x == c))
        {
            return Err(CapabilityError::message(
                "unsupported MacParakeet source database: identity/title/date columns missing",
            ));
        }
        let wanted = [
            "id",
            "fileName",
            "titleOverride",
            "sourceType",
            "createdAt",
            "durationMs",
            "filePath",
            "sourceURL",
            "channelName",
            "rawTranscript",
            "cleanTranscript",
            "transcriptSegments",
            "wordTimestamps",
            "diarizationSegments",
            "speakers",
            "engine",
            "engineVariant",
        ];
        let projection = wanted
            .iter()
            .map(|name| {
                if columns.iter().any(|c| c == name) {
                    format!("\"{name}\"")
                } else {
                    format!("NULL AS \"{name}\"")
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let query = include_str!("catalog.sql").replace("{{projection}}", &projection);
        let rows = sqlx::query(&query)
            .fetch_all(&mut *tx)
            .await
            .map_err(CapabilityError::new)?;
        let mut out = vec![];
        for row in rows {
            let raw: Vec<u8> = row
                .try_get("id")
                .or_else(|_| row.try_get::<String, _>("id").map(String::into_bytes))
                .map_err(CapabilityError::new)?;
            let id = if raw.len() == 16 {
                uuid::Uuid::from_slice(&raw)
                    .map_err(CapabilityError::new)?
                    .to_string()
            } else {
                String::from_utf8(raw)
                    .map_err(CapabilityError::new)?
                    .to_lowercase()
            };
            let get = |name: &str| row.try_get::<Option<String>, _>(name).ok().flatten();
            let source_type = get("sourceType").unwrap_or_else(|| "file".into());
            let mut original = serde_json::Map::new();
            original.insert("id".into(), serde_json::json!(id));
            for name in wanted.iter().filter(|n| **n != "id") {
                if *name == "durationMs" {
                    original.insert(
                        name.to_string(),
                        serde_json::json!(row.try_get::<Option<i64>, _>(*name).ok().flatten()),
                    );
                } else {
                    original.insert(name.to_string(), serde_json::json!(get(name)));
                }
            }
            out.push(LegacySourceRecord {
                source_id: id,
                source_type,
                title: get("titleOverride")
                    .or_else(|| get("fileName"))
                    .unwrap_or_else(|| "Untitled imported recording".into()),
                created_at: get("createdAt").unwrap_or_else(|| "unknown".into()),
                duration_ms: row.try_get::<Option<i64>, _>("durationMs").ok().flatten(),
                file_path: get("filePath"),
                original: serde_json::Value::Object(original),
            });
        }
        tx.rollback().await.map_err(CapabilityError::new)?;
        pool.close().await;
        Ok(out)
    }
}
