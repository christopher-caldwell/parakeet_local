UPDATE runs SET status=?2,data=CASE
WHEN json_extract(data,'$.configuration.cancellation_requested_at') IS NOT NULL
THEN json_set(?3,'$.configuration.cancellation_requested_at',json_extract(data,'$.configuration.cancellation_requested_at'))
ELSE ?3 END
WHERE id=?1 AND status='running'
AND (json_extract(data,'$.configuration.cancellation_requested_at') IS NULL OR ?2='interrupted')
