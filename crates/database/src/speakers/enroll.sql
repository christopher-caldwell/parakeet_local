INSERT INTO voice_examples(id,profile_id,data) VALUES(?1,?2,?3) ON CONFLICT(id) DO NOTHING
