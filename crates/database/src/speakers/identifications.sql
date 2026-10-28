SELECT data FROM cluster_identifications WHERE (?1 IS NULL OR recording_id=?1) ORDER BY rowid
