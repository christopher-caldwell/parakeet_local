CREATE TABLE recordings (
 id TEXT PRIMARY KEY NOT NULL, source_key TEXT UNIQUE NOT NULL,
 source_kind TEXT NOT NULL, source_title TEXT NOT NULL, title_override TEXT,
 created_at TEXT NOT NULL, duration_ms INTEGER CHECK(duration_ms IS NULL OR duration_ms>=0), metadata TEXT NOT NULL CHECK(json_valid(metadata)),
 original TEXT NOT NULL CHECK(json_valid(original))
);
CREATE TABLE tracks (
 id TEXT PRIMARY KEY NOT NULL, recording_id TEXT NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
 data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE runs (
 id TEXT PRIMARY KEY NOT NULL, recording_id TEXT NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
 kind TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('running','completed','failed','interrupted')), data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE segments (
 id TEXT PRIMARY KEY NOT NULL, recording_id TEXT NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
 run_id TEXT NOT NULL, start_ms INTEGER, ordinal INTEGER NOT NULL, data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX segments_recording_time ON segments(recording_id,start_ms,ordinal);
CREATE TABLE speaker_profiles(id TEXT PRIMARY KEY NOT NULL,name TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE TABLE voice_examples(id TEXT PRIMARY KEY NOT NULL,profile_id TEXT NOT NULL REFERENCES speaker_profiles(id) ON DELETE CASCADE,data TEXT NOT NULL CHECK(json_valid(data)));
CREATE TABLE corrections(sequence INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE NOT NULL,segment_id TEXT NOT NULL REFERENCES segments(id),profile_id TEXT REFERENCES speaker_profiles(id) ON DELETE SET NULL,data TEXT NOT NULL CHECK(json_valid(data)));
CREATE INDEX corrections_segment ON corrections(segment_id,sequence);
CREATE TABLE prompt_profiles(id TEXT PRIMARY KEY NOT NULL,bundle_id TEXT NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)));
CREATE TABLE shortcuts(id TEXT PRIMARY KEY NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)));
