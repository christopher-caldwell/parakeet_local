SELECT id, source_key, source_kind, source_title, title_override, created_at, duration_ms, metadata FROM recordings
WHERE ?1 IS NULL OR instr(lower(COALESCE(title_override,source_title)),lower(?1))>0 OR id IN (SELECT recording_id FROM segments WHERE instr(lower(json_extract(data,'$.text')),lower(?1))>0)
ORDER BY created_at DESC,id
