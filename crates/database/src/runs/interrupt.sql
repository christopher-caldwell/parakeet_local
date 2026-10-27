UPDATE runs SET status='interrupted',data=CASE
WHEN json_extract(data,'$.configuration.cancellation_requested_at') IS NOT NULL
THEN json_set(?2,'$.configuration.cancellation_requested_at',json_extract(data,'$.configuration.cancellation_requested_at'))
ELSE ?2 END
WHERE id=?1 AND status='running' AND json_extract(data,'$.configuration.ownership_protocol')='file-lock-v1'
