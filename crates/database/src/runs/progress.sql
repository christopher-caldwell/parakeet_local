UPDATE runs SET data=json_set(data,'$.result',json_set(coalesce(json_extract(data,'$.result'),'{}'),'$.progress',json(?2)))
WHERE id=?1 AND status='running' AND json_extract(data,'$.configuration.control_protocol')='processing-control-v1'
AND coalesce(json_extract(data,'$.result.progress.sequence'),-1)<json_extract(?2,'$.sequence')
