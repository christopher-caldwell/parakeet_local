SELECT id, source_key, source_kind, source_title, title_override, created_at, duration_ms, metadata FROM recordings
WHERE ?1 IS NULL OR instr(lower(COALESCE(title_override,source_title)),lower(?1))>0
 OR id IN (
 SELECT s.recording_id FROM current_transcript_segments s
 -- Every correction publication stores the effective text snapshot, including
 -- inherited text on identity changes and the prior text on Undo.
 LEFT JOIN corrections c ON c.sequence=(SELECT MAX(n.sequence) FROM corrections n WHERE n.segment_id=s.id)
 WHERE instr(lower(coalesce(json_extract(c.data,'$.replacement_text'),json_extract(s.data,'$.text'))),lower(?1))>0
 )
ORDER BY created_at DESC,id
