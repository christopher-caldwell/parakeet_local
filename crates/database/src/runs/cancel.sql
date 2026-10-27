UPDATE runs SET data=json_set(data,'$.configuration.cancellation_requested_at',coalesce(json_extract(data,'$.configuration.cancellation_requested_at'),?2))
WHERE id=?1 AND status='running' AND json_extract(data,'$.configuration.control_protocol')='processing-control-v1'
