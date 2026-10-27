CREATE TABLE audio_preferences(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL CHECK(json_valid(data)));
CREATE TABLE dictations(id TEXT PRIMARY KEY NOT NULL,recording_id TEXT NOT NULL REFERENCES recordings(id),data TEXT NOT NULL CHECK(json_valid(data)));
