SELECT data FROM voice_examples WHERE json_extract(data,'$.recording_id')=?1 ORDER BY id
