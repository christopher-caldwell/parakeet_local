CREATE TABLE speaker_suggestions (
 id TEXT PRIMARY KEY NOT NULL,
 recording_id TEXT NOT NULL REFERENCES recordings(id),
 data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE suggestion_resolutions (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT UNIQUE NOT NULL,
 suggestion_id TEXT NOT NULL REFERENCES speaker_suggestions(id),
 profile_id TEXT REFERENCES speaker_profiles(id) ON DELETE SET NULL,
 data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX suggestions_recording ON speaker_suggestions(recording_id);
CREATE INDEX resolutions_suggestion ON suggestion_resolutions(suggestion_id,sequence);
