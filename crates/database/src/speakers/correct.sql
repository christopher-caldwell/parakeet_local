INSERT INTO corrections(id,segment_id,profile_id,data)
SELECT ?1,?2,?3,?4 WHERE (SELECT COUNT(*) FROM corrections WHERE segment_id=?2)=?5
