SELECT data FROM current_transcript_segments WHERE recording_id=?1
ORDER BY start_ms IS NULL,start_ms,ordinal,id
