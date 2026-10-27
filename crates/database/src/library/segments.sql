SELECT data FROM segments WHERE recording_id=?1 AND (
 run_id IN (
   SELECT r.id FROM runs r WHERE r.recording_id=?1 AND r.status='completed' AND r.kind='transcription'
   AND r.rowid=(SELECT MAX(n.rowid) FROM runs n WHERE n.recording_id=r.recording_id AND n.status='completed' AND n.kind='transcription' AND json_extract(n.data,'$.configuration.track.id') IS json_extract(r.data,'$.configuration.track.id'))
 ) OR (run_id='import:'||?1 AND NOT EXISTS (SELECT 1 FROM runs WHERE recording_id=?1 AND kind='transcription' AND status='completed'))
) ORDER BY start_ms IS NULL,start_ms,ordinal,id
