UPDATE recordings
SET metadata = json_set(metadata, '$.meeting_finalization', json(?3))
WHERE id = ?1 AND json_type(metadata) = 'object'
AND json_extract(metadata, '$.meeting_finalization') IS json(?2)
