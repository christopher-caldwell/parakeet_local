UPDATE cluster_identifications SET data=?2 WHERE id=?1 AND json_extract(data,'$.undone_at') IS NULL
