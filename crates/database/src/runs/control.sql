SELECT status, json_extract(data,'$.configuration.cancellation_requested_at') AS "cancellation_requested_at: String" FROM runs WHERE id=?1
