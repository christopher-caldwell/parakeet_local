WITH speaker_recordings AS MATERIALIZED (
 SELECT DISTINCT json_extract(data,'$.recording_id') AS recording_id FROM voice_examples
), corrected_recordings AS MATERIALIZED (
 SELECT DISTINCT s.recording_id FROM corrections c JOIN segments s ON s.id=c.segment_id
)
SELECT r.id,r.recording_id,r.status,json_extract(r.data,'$.completed_at') AS "completed_at?: String",
 coalesce(json_extract(r.data,'$.configuration.ownership_protocol'),'')='file-lock-v1' AS "ownership_verified!: bool",
 CASE WHEN (json_type(m.metadata,'$.meeting_finalization') IS NOT NULL
 AND (coalesce(json_extract(m.metadata,'$.meeting_finalization.status'),'unknown')!='completed'
 OR (json_type(m.metadata,'$.meeting_finalization.speaker_analysis')='object'
 AND coalesce(json_extract(m.metadata,'$.meeting_analysis.status'),'unknown')!='completed')))
 OR (json_type(m.metadata,'$.meeting_analysis') IS NOT NULL
 AND coalesce(json_extract(m.metadata,'$.meeting_analysis.status'),'unknown')!='completed') THEN 1 ELSE 0 END AS "pending_meeting!: bool",
 v.recording_id IS NOT NULL AS "speaker_reference!: bool",
 c.recording_id IS NOT NULL AS "manual_correction!: bool"
FROM runs r JOIN recordings m ON m.id=r.recording_id
LEFT JOIN speaker_recordings v ON v.recording_id=r.recording_id
LEFT JOIN corrected_recordings c ON c.recording_id=r.recording_id
ORDER BY r.id
