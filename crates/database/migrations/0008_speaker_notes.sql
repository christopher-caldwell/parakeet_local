CREATE TABLE speaker_notes (
 profile_id TEXT PRIMARY KEY NOT NULL REFERENCES speaker_profiles(id) ON DELETE CASCADE,
 notes TEXT NOT NULL
);
