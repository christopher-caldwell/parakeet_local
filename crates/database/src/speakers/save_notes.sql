INSERT INTO speaker_notes(profile_id,notes) SELECT id,?2 FROM speaker_profiles WHERE id=?1 ON CONFLICT(profile_id) DO UPDATE SET notes=excluded.notes
