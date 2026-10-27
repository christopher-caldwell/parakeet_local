INSERT INTO suggestion_resolutions(id,suggestion_id,profile_id,data)
SELECT ?1,?2,?3,?4 WHERE (SELECT COUNT(*) FROM suggestion_resolutions WHERE suggestion_id=?2)=?5
