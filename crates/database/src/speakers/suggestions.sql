SELECT data FROM speaker_suggestions WHERE (?1 IS NULL OR recording_id=?1) ORDER BY rowid
