UPDATE runs SET status='interrupted',data=?2
WHERE id=?1 AND status='running' AND json_extract(data,'$.configuration.ownership_protocol')='file-lock-v1'
