INSERT INTO cleanup_reviews(id,data,updated_at) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET data=excluded.data, updated_at=excluded.updated_at
