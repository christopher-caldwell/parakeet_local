# Local data boundary

Local imports, decoding, ASR, diarization, embeddings, and benchmarks never send recording content to a service. Dependency and explicitly requested model downloads are allowed. Media retrieval is an explicit URL operation. AI enhancement requires both an enabled profile and explicit authorization of its declared local/remote subprocess route; neither installation nor normal dictation grants consent. Profiles use bundle IDs and no approved prompt text is fabricated.

Keep `.local/`, `.references/`, models, private exports, recordings and derived clips out of Git. Diagnostics should favor counts/timings over transcript content. Speaker vectors stay local and are excluded from AI transformation input. Originals are opened read-only and never migrated or repaired.
