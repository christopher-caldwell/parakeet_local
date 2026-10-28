-- Full-source results publish independently, or only after every declared source
-- in a coordinated batch completes. Bounded experiments remain in run history.
CREATE VIEW current_transcript_segments AS
WITH eligible AS MATERIALIZED (
 SELECT r.rowid AS position,r.id,r.recording_id,json_extract(r.data,'$.configuration.track.id') AS track_id
 FROM runs r
 WHERE r.status='completed' AND r.kind='transcription'
 AND coalesce(json_extract(r.data,'$.configuration.request.start_ms'),0)=0
 AND json_extract(r.data,'$.configuration.request.duration_ms') IS NULL
 AND (
  json_type(r.data,'$.configuration.publication_group') IS NULL
  OR json_type(r.data,'$.configuration.publication_group')='null'
  OR (
   json_type(r.data,'$.configuration.publication_group.track_ids')='array'
   AND json_array_length(r.data,'$.configuration.publication_group.track_ids')>0
   AND NOT EXISTS (
    SELECT 1 FROM json_each(r.data,'$.configuration.publication_group.track_ids') expected
    WHERE NOT EXISTS (
     SELECT 1 FROM runs peer WHERE peer.recording_id=r.recording_id
     AND peer.kind='transcription' AND peer.status='completed'
     AND json_extract(peer.data,'$.configuration.publication_group.id')=json_extract(r.data,'$.configuration.publication_group.id')
     AND json_extract(peer.data,'$.configuration.track.id')=expected.value
     AND coalesce(json_extract(peer.data,'$.configuration.request.start_ms'),0)=0
     AND json_extract(peer.data,'$.configuration.request.duration_ms') IS NULL
    )
   )
  )
 )
), selected AS (
 SELECT e.id FROM eligible e WHERE e.position=(SELECT MAX(n.position) FROM eligible n WHERE n.recording_id=e.recording_id AND n.track_id IS e.track_id)
)
SELECT s.* FROM segments s WHERE s.run_id IN (SELECT id FROM selected)
 OR (s.run_id='import:'||s.recording_id AND NOT EXISTS (SELECT 1 FROM eligible e WHERE e.recording_id=s.recording_id));
