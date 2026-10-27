-- The projection is generated only from hard-coded known source columns after read-only schema inspection.
-- Source MacParakeet schemas are external and version-dependent; this query cannot be checked against our schema.
SELECT {{projection}} FROM transcriptions ORDER BY createdAt,id
