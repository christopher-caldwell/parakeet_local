UPDATE storage_policy SET revision=revision+1,data=?1,updated_at=?2
WHERE id=1 AND revision=?3 RETURNING revision,data,updated_at
