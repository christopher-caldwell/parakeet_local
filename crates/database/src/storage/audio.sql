SELECT id,recording_id,json_extract(data,'$.path') AS "path!: String" FROM tracks ORDER BY id
