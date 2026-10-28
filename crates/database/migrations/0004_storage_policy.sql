CREATE TABLE storage_policy(
 id INTEGER PRIMARY KEY CHECK(id=1),
 revision INTEGER NOT NULL CHECK(revision>=0),
 data TEXT NOT NULL CHECK(json_valid(data)),
 updated_at TEXT
);
INSERT INTO storage_policy(id,revision,data) VALUES(1,0,'{"completed_scratch_after_days":null}');
