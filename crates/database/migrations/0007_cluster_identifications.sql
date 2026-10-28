CREATE TABLE cluster_identifications (
 id TEXT PRIMARY KEY NOT NULL,
 recording_id TEXT NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
 profile_id TEXT REFERENCES speaker_profiles(id) ON DELETE SET NULL,
 data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX cluster_identifications_recording ON cluster_identifications(recording_id);
