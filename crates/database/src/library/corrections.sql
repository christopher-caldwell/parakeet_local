SELECT c.data,c.profile_id FROM corrections c JOIN segments s ON s.id=c.segment_id WHERE s.recording_id=?1 ORDER BY c.sequence
