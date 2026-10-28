UPDATE recordings SET metadata=json_set(
 CASE WHEN json_extract(metadata,'$.meeting_analysis.id') IS NOT NULL
 AND json_extract(metadata,'$.meeting_analysis.id')!=json_extract(?3,'$.id')
 THEN json_insert(json_set(metadata,'$.analysis_history',json(coalesce(json_extract(metadata,'$.analysis_history'),'[]'))),'$.analysis_history[#]',json_extract(metadata,'$.meeting_analysis'))
 ELSE metadata END,
 '$.meeting_analysis',json(?3))
WHERE id=?1 AND json_type(metadata)='object'
AND json_extract(metadata,'$.meeting_analysis') IS json(?2)
