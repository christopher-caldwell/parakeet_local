INSERT INTO prompt_profiles(id,bundle_id,data) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET bundle_id=excluded.bundle_id,data=excluded.data
