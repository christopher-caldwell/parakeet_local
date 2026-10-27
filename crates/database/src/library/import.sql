INSERT INTO recordings(id,source_key,source_kind,source_title,title_override,created_at,duration_ms,metadata,original) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(source_key) DO NOTHING
