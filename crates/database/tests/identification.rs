use parakeet_application::{
    LibraryWrites,
    processing::{EmbeddingOutput, EmbeddingRequest, RunWrites, VoiceEmbeddings},
    speakers::{
        CorrectSegmentInput, IdentifyClusterInput, PersonChoice, ResolveSuggestionInput,
        SpeakerQueries, SpeakerService, SpeakerWrites, SuggestSpeakerInput,
    },
};
use parakeet_database::*;
use parakeet_domain::*;
use serde_json::json;
use std::sync::Arc;

struct FixtureModels;
#[async_trait::async_trait]
impl parakeet_application::LocalModels for FixtureModels {
    async fn selection(
        &self,
    ) -> Result<parakeet_application::ModelSelection, parakeet_application::CapabilityError> {
        Ok(Default::default())
    }
    async fn list(
        &self,
    ) -> Result<Vec<parakeet_application::ModelInfo>, parakeet_application::CapabilityError> {
        Ok(vec![])
    }
    async fn install(
        &self,
        _: &str,
    ) -> Result<parakeet_application::ModelInfo, parakeet_application::CapabilityError> {
        Err(parakeet_application::CapabilityError::message(
            "test does not install",
        ))
    }
    async fn verify(
        &self,
        id: &str,
    ) -> Result<parakeet_application::ModelInfo, parakeet_application::CapabilityError> {
        Ok(parakeet_application::ModelInfo {
            experimental_word_timestamps: false,
            id: id.into(),
            category: "asr".into(),
            revision: "test".into(),
            installed: true,
            verified: true,
            bytes: Some(1),
            license: "test".into(),
            path: None,
            detail: json!({}),
        })
    }
    async fn select(&self, _: &str) -> Result<(), parakeet_application::CapabilityError> {
        Ok(())
    }
    async fn select_embedding(&self, _: &str) -> Result<(), parakeet_application::CapabilityError> {
        Ok(())
    }
    async fn remove(&self, _: &str) -> Result<(), parakeet_application::CapabilityError> {
        Ok(())
    }
}
struct FixtureEmbeddings;
#[async_trait::async_trait]
impl VoiceEmbeddings for FixtureEmbeddings {
    async fn embed(
        &self,
        _: &EmbeddingRequest,
    ) -> Result<EmbeddingOutput, parakeet_application::CapabilityError> {
        Ok(EmbeddingOutput {
            model: "test".into(),
            revision: "1".into(),
            vector: vec![1., 0.],
            metrics: json!({"mock":true}),
        })
    }
}
fn segment(id: &str, start: i64, end: i64) -> TranscriptSegment {
    TranscriptSegment {
        id: id.into(),
        recording_id: "r".into(),
        run_id: "import:r".into(),
        track_id: Some("track".into()),
        ordinal: 0,
        start_ms: Some(start),
        end_ms: Some(end),
        text: format!("words {id}"),
        speaker_claim: None,
        evidence: json!({}),
    }
}
fn interval(speaker: &str, start: i64, end: i64) -> SpeakerInterval {
    SpeakerInterval {
        start_ms: start,
        end_ms: end,
        speaker: speaker.into(),
        track_id: Some("track".into()),
    }
}
async fn fixture() -> (tempfile::TempDir, Arc<SqliteStore>, SpeakerService) {
    fixture_with(vec![
        segment("s1", 0, 6_000),
        segment("s2", 7_000, 13_000),
        segment("s3", 14_000, 20_000),
        segment("s4", 30_000, 36_000),
    ])
    .await
}
async fn fixture_with(
    segments: Vec<TranscriptSegment>,
) -> (tempfile::TempDir, Arc<SqliteStore>, SpeakerService) {
    fixture_with_overlap(segments, false).await
}
async fn fixture_with_overlap(
    segments: Vec<TranscriptSegment>,
    microphone: bool,
) -> (tempfile::TempDir, Arc<SqliteStore>, SpeakerService) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.db");
    create_empty(&path).await.unwrap();
    migrate(&path).await.unwrap();
    let store = Arc::new(SqliteStore::new(open_existing(&path).await.unwrap()));
    let mut bundle = ImportBundle {
        recording: Recording {
            id: "r".into(),
            source_key: "original:r".into(),
            source_kind: "meeting".into(),
            source_title: "Meeting".into(),
            title_override: None,
            created_at: "2026-01-01".into(),
            duration_ms: Some(60_000),
            metadata: json!({}),
        },
        tracks: vec![AudioTrack {
            id: "track".into(),
            recording_id: "r".into(),
            kind: TrackKind::System,
            path: "/missing/source.m4a".into(),
            start_offset_ms: Some(0),
            sample_rate: Some(48000),
            duration_ms: Some(60_000),
            available: true,
            size_bytes: None,
            modified_unix_ms: None,
            provenance: json!({}),
        }],
        segments,
        original: json!({}),
        warnings: vec![],
    };
    if microphone {
        let mut mic = bundle.tracks[0].clone();
        mic.id = "mic-track".into();
        mic.kind = TrackKind::Microphone;
        bundle.tracks.push(mic);
    }
    store.import(&bundle).await.unwrap();
    let intervals = vec![
        interval("a", 0, 6_500),
        interval("a", 7_000, 13_500),
        interval("b", 14_000, 20_500),
        interval("a", 30_000, 36_000),
        interval("b", 33_000, 37_000),
    ];
    let mut run = ProcessingRun {
        id: "diarization-full".into(),
        recording_id: "r".into(),
        kind: "diarization".into(),
        model: "fixture".into(),
        model_revision: "1".into(),
        status: "running".into(),
        started_at: "now".into(),
        completed_at: None,
        configuration: json!({"track":{"id":"track"},"request":{"start_ms":0,"duration_ms":null}}),
        result: json!({}),
        error: None,
    };
    store.start(&run).await.unwrap();
    run.status = "completed".into();
    run.result = json!({"intervals":intervals});
    store.finish(&run, &[]).await.unwrap();
    if microphone {
        let mut mic = run.clone();
        mic.id = "mic-diarization".into();
        mic.configuration["track"]["id"] = json!("mic-track");
        for interval in mic.result["intervals"].as_array_mut().unwrap() {
            interval["track_id"] = json!("mic-track");
        }
        mic.status = "running".into();
        store.start(&mic).await.unwrap();
        mic.status = "completed".into();
        store.finish(&mic, &[]).await.unwrap();
    }
    let service = SpeakerService::new(
        store.clone(),
        store.clone(),
        store.clone(),
        Arc::new(FixtureEmbeddings),
        Arc::new(FixtureModels),
    );
    (tmp, store, service)
}
fn identify(profile: PersonChoice) -> IdentifyClusterInput {
    IdentifyClusterInput {
        // The user played every clip of the cluster.
        heard: vec![(0, 10_000_000)],
        recording_id: "r".into(),
        track_id: "track".into(),
        run_id: "diarization-full".into(),
        cluster: "a".into(),
        person: profile,
        confirmed: true,
    }
}
#[tokio::test]
async fn naming_a_cluster_corrects_clean_segments_admits_examples_and_undoes_as_one_unit() {
    let (_tmp, store, service) = fixture().await;
    let mut unconfirmed = identify(PersonChoice::New("Person".into()));
    unconfirmed.confirmed = false;
    assert!(service.identify_cluster(unconfirmed).await.is_err());
    assert!(service.profiles().await.unwrap().is_empty());

    let named = service
        .identify_cluster(identify(PersonChoice::New("Person".into())))
        .await
        .unwrap();
    let ids: Vec<_> = named
        .segments
        .iter()
        .map(|s| s.segment_id.as_str())
        .collect();
    assert_eq!(ids, ["s1", "s2"]);
    // s4 is shared with another voice: listed, never named.
    assert_eq!(named.skipped.len(), 1);
    assert_eq!(named.skipped[0].segment_id, "s4");
    assert_eq!(named.skipped[0].reason, "ambiguous_or_overlapping");
    assert!(store.corrections("s3").await.unwrap().is_empty());
    for id in ["s1", "s2"] {
        let history = store.corrections(id).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0].profile_id.as_deref(),
            Some(named.profile_id.as_str())
        );
    }
    // Two clean turns were admitted as verified, reversible, inspectable examples.
    let examples = service.examples(Some(&named.profile_id)).await.unwrap();
    assert_eq!(examples.len(), 2);
    for example in &examples {
        let origin = example.origin.as_ref().unwrap();
        assert_eq!(origin.verified_by, "user_identification");
        assert_eq!(origin.source_kind, "cluster_identification");
        assert_eq!(origin.source_id, named.id);
    }
    assert_eq!(service.identifications(Some("r")).await.unwrap().len(), 1);

    let report = service.undo_identification(&named.id).await.unwrap();
    assert_eq!(report.reverted_segments, 2);
    assert_eq!(report.removed_examples, 2);
    assert!(report.left_in_place.is_empty());
    for id in ["s1", "s2"] {
        let history = store.corrections(id).await.unwrap();
        assert_eq!(history.last().unwrap().action, "undo");
        assert!(history.last().unwrap().profile_id.is_none());
    }
    assert!(service.examples(None).await.unwrap().is_empty());
    assert!(service.undo_identification(&named.id).await.is_err());
}
#[tokio::test]
async fn later_edits_survive_undo_and_a_superseded_run_cannot_be_named() {
    let (_tmp, store, service) = fixture().await;
    let person = service.create("Person").await.unwrap();
    let named = service
        .identify_cluster(identify(PersonChoice::Existing(person.id.clone())))
        .await
        .unwrap();
    service
        .correct(CorrectSegmentInput {
            segment_id: "s1".into(),
            profile_id: None,
            text: Some("edited".into()),
            action: "text".into(),
        })
        .await
        .unwrap();
    let report = service.undo_identification(&named.id).await.unwrap();
    assert_eq!(report.left_in_place, vec!["s1".to_owned()]);
    assert_eq!(report.reverted_segments, 1);
    assert_eq!(
        store
            .corrections("s1")
            .await
            .unwrap()
            .last()
            .unwrap()
            .action,
        "text"
    );

    // A bounded experiment never replaces the full run; naming a stale run id is refused.
    let mut stale = identify(PersonChoice::Existing(person.id));
    stale.run_id = "not-current".into();
    assert!(service.identify_cluster(stale).await.is_err());
}
#[tokio::test]
async fn assigning_a_clean_segment_admits_an_example_and_undo_removes_it() {
    let (_tmp, _store, service) = fixture().await;
    let person = service.create("Person").await.unwrap();
    let outcome = service
        .correct(CorrectSegmentInput {
            segment_id: "s3".into(),
            profile_id: Some(person.id.clone()),
            text: None,
            action: "assign".into(),
        })
        .await
        .unwrap();
    let example = outcome.example.expect("clean segment admits an example");
    assert_eq!(
        example.origin.as_ref().unwrap().source_kind,
        "segment_correction"
    );
    assert_eq!(service.examples(Some(&person.id)).await.unwrap().len(), 1);
    // An overlapped segment is assigned without admitting anything.
    let overlapped = service
        .correct(CorrectSegmentInput {
            segment_id: "s4".into(),
            profile_id: Some(person.id.clone()),
            text: None,
            action: "assign".into(),
        })
        .await
        .unwrap();
    assert!(overlapped.example.is_none());
    assert!(overlapped.example_note.is_some());
    service.undo("s3").await.unwrap();
    assert!(service.examples(Some(&person.id)).await.unwrap().is_empty());
    // Rejections never enroll.
    let rejected = service
        .correct(CorrectSegmentInput {
            segment_id: "s1".into(),
            profile_id: None,
            text: None,
            action: "reject".into(),
        })
        .await
        .unwrap();
    assert!(rejected.example.is_none());
    assert!(service.examples(None).await.unwrap().is_empty());
}
#[tokio::test]
async fn accepting_a_suggestion_admits_a_reversible_example_and_rejecting_admits_none() {
    let (_tmp, _store, service) = fixture().await;
    let named = service
        .identify_cluster(identify(PersonChoice::New("Person".into())))
        .await
        .unwrap();
    let before = service
        .examples(Some(&named.profile_id))
        .await
        .unwrap()
        .len();
    let suggestion = service
        .suggest(SuggestSpeakerInput {
            recording_id: "r".into(),
            track_id: "track".into(),
            start_ms: 14_000,
            end_ms: 20_500,
            more_ranges: vec![],
            policy: parakeet_domain::recognition::MatchPolicy {
                minimum_score: 0.5,
                minimum_margin: 0.0,
                minimum_examples: 1,
            },
        })
        .await
        .unwrap();
    assert_eq!(
        suggestion.decision.suggested_profile.as_deref(),
        Some(named.profile_id.as_str())
    );
    // A prediction alone enrolls nothing.
    assert_eq!(
        service
            .examples(Some(&named.profile_id))
            .await
            .unwrap()
            .len(),
        before
    );
    let accepted = service
        .resolve_suggestion(ResolveSuggestionInput {
            id: suggestion.id.clone(),
            action: "accept".into(),
            segment_id: Some("s3".into()),
            profile_id: None,
        })
        .await
        .unwrap();
    assert!(accepted.admitted_example_id.is_some());
    assert_eq!(
        service
            .examples(Some(&named.profile_id))
            .await
            .unwrap()
            .len(),
        before + 1
    );
    service
        .resolve_suggestion(ResolveSuggestionInput {
            id: suggestion.id,
            action: "undo".into(),
            segment_id: None,
            profile_id: None,
        })
        .await
        .unwrap();
    assert_eq!(
        service
            .examples(Some(&named.profile_id))
            .await
            .unwrap()
            .len(),
        before
    );
}
fn imported(id: &str, start: i64, end: i64, claim: &str) -> TranscriptSegment {
    let mut s = segment(id, start, end);
    s.speaker_claim = Some(claim.into());
    s.evidence = json!({"origin":"imported_unverified"});
    s
}
#[tokio::test]
async fn imported_labels_only_propose_clean_clips_and_enroll_nothing() {
    let (_tmp, _store, service) = fixture_with(vec![
        imported("a1", 0, 6_000, "Alex"),
        imported("a2", 10_000, 16_000, "Alex"),
        imported("a3", 15_000, 21_000, "Sam"),
        imported("o1", 30_000, 36_000, "Others 1"),
        imported("m1", 40_000, 46_000, "Me"),
        imported("short", 50_000, 51_000, "Alex"),
    ])
    .await;
    let candidates = service.imported_label_candidates("r").await.unwrap();
    let names: Vec<_> = candidates.iter().map(|c| c.claim.as_str()).collect();
    // Sam overlaps Alex's second clip, so neither overlapping clip is offered.
    assert_eq!(names, ["Alex", "Me"]);
    // a2 overlaps another label on the same source, and the one-second clip is too short.
    let alex = &candidates[0];
    assert_eq!(
        alex.clips
            .iter()
            .map(|c| c.segment_id.as_str())
            .collect::<Vec<_>>(),
        ["a1"]
    );
    assert!(candidates[1].is_self);
    assert!(candidates.iter().all(|c| c.claim != "Others 1"));
    assert!(service.examples(None).await.unwrap().is_empty());
}
#[tokio::test]
async fn reembedding_replaces_old_revisions_and_reports_missing_audio() {
    let (_tmp, store, service) = fixture().await;
    let person = service.create("Person").await.unwrap();
    let old = VoiceExample {
        id: "old".into(),
        profile_id: person.id.clone(),
        recording_id: "r".into(),
        track_id: "track".into(),
        start_ms: 0,
        end_ms: 5_000,
        model: "test".into(),
        revision: "0".into(),
        capture_domain: "system".into(),
        vector: vec![0., 1.],
        verified_at: "then".into(),
        origin: None,
    };
    let current = VoiceExample {
        id: "current".into(),
        revision: "1".into(),
        start_ms: 7_000,
        end_ms: 12_000,
        ..old.clone()
    };
    for example in [&old, &current] {
        store.enroll(example).await.unwrap();
    }
    let report = service.reembed(None).await.unwrap();
    assert_eq!(report.replaced, 1);
    assert_eq!(report.already_current, 1);
    let examples = service.examples(None).await.unwrap();
    assert_eq!(examples.len(), 2);
    assert!(examples.iter().all(|e| e.revision == "1" && e.id != "old"));
    assert!(
        examples
            .iter()
            .all(|e| e.verified_at == "then" || e.id == "current")
    );
}
#[tokio::test]
async fn profile_history_lists_enrollments_recognitions_and_notes() {
    let (_tmp, _store, service) = fixture().await;
    let named = service
        .identify_cluster(identify(PersonChoice::New("Person".into())))
        .await
        .unwrap();
    service
        .save_notes(&named.profile_id, "met at standup")
        .await
        .unwrap();
    let suggestion = service
        .suggest(SuggestSpeakerInput {
            recording_id: "r".into(),
            track_id: "track".into(),
            start_ms: 14_000,
            end_ms: 20_500,
            more_ranges: vec![],
            policy: parakeet_domain::recognition::MatchPolicy {
                minimum_score: 0.5,
                minimum_margin: 0.0,
                minimum_examples: 1,
            },
        })
        .await
        .unwrap();
    let before = service.history(&named.profile_id).await.unwrap();
    assert_eq!(before.recognitions[0].outcome, "suggested");
    service
        .resolve_suggestion(ResolveSuggestionInput {
            id: suggestion.id,
            action: "accept".into(),
            segment_id: Some("s3".into()),
            profile_id: None,
        })
        .await
        .unwrap();
    let after = service.history(&named.profile_id).await.unwrap();
    assert_eq!(after.notes, "met at standup");
    assert_eq!(after.recognitions[0].outcome, "accepted");
    assert_eq!(after.enrollments.len(), 3);
    assert!(service.save_notes("missing", "x").await.is_err());
}

#[tokio::test]
async fn simultaneous_microphone_and_system_voices_remain_available_to_name() {
    let (_tmp, _store, service) = fixture_with_overlap(vec![], true).await;
    let candidates = service.candidates("r").await.unwrap();
    assert_eq!(
        candidates.iter().filter(|c| c.track_id == "track").count(),
        2
    );
    assert_eq!(
        candidates
            .iter()
            .filter(|c| c.track_id == "mic-track")
            .count(),
        2,
        "complete temporal overlap is not proof of echo and must not discard a real microphone voice"
    );
}
fn assign(segment: &str, profile: Option<&str>, action: &str) -> CorrectSegmentInput {
    CorrectSegmentInput {
        segment_id: segment.into(),
        profile_id: profile.map(str::to_owned),
        text: None,
        action: action.into(),
    }
}
#[tokio::test]
async fn reassigning_a_segment_moves_its_example_and_undo_moves_it_back() {
    let (_tmp, _store, service) = fixture().await;
    let a = service.create("Person A").await.unwrap();
    let b = service.create("Person B").await.unwrap();
    service
        .correct(assign("s3", Some(&a.id), "assign"))
        .await
        .unwrap();
    assert_eq!(service.examples(Some(&a.id)).await.unwrap().len(), 1);
    // The user says it was B after all: A must not keep B's voice.
    let moved = service
        .correct(assign("s3", Some(&b.id), "assign"))
        .await
        .unwrap();
    assert!(moved.example.is_some());
    assert!(service.examples(Some(&a.id)).await.unwrap().is_empty());
    assert_eq!(service.examples(Some(&b.id)).await.unwrap().len(), 1);
    // Undo restores A's decision and its example; B's goes.
    service.undo("s3").await.unwrap();
    assert_eq!(service.examples(Some(&a.id)).await.unwrap().len(), 1);
    assert!(service.examples(Some(&b.id)).await.unwrap().is_empty());
    // Unknown removes every example claiming that speech.
    service
        .correct(assign("s3", None, "unknown"))
        .await
        .unwrap();
    assert!(service.examples(None).await.unwrap().is_empty());
}
#[tokio::test]
async fn naming_a_cluster_never_learns_from_speech_assigned_to_someone_else() {
    let (_tmp, _store, service) = fixture().await;
    let other = service.create("Someone else").await.unwrap();
    // s2 sits in cluster "a" but the user already said it is someone else.
    service
        .correct(assign("s2", Some(&other.id), "assign"))
        .await
        .unwrap();
    let named = service
        .identify_cluster(identify(PersonChoice::New("Person".into())))
        .await
        .unwrap();
    assert!(
        named
            .skipped
            .iter()
            .any(|s| s.segment_id == "s2" && s.reason == "already_corrected")
    );
    let examples = service.examples(Some(&named.profile_id)).await.unwrap();
    assert_eq!(examples.len(), 1);
    assert!(
        examples
            .iter()
            .all(|e| e.end_ms <= 7_000 || e.start_ms >= 13_500)
    );
}
#[tokio::test]
async fn naming_admits_only_the_clips_the_user_heard() {
    let (_tmp, _store, service) = fixture().await;
    let mut input = identify(PersonChoice::New("Person".into()));
    input.heard = vec![];
    let named = service.identify_cluster(input).await.unwrap();
    // Nothing explicitly played: only the clip offered first (the longest clean turn).
    assert_eq!(named.example_ids.len(), 1);
}
#[tokio::test]
async fn retracting_a_name_removes_every_example_learned_from_its_lines() {
    let (_tmp, _store, service) = fixture().await;
    let other = service.create("Person B").await.unwrap();
    let named = service
        .identify_cluster(identify(PersonChoice::New("Person".into())))
        .await
        .unwrap();
    assert_eq!(
        service
            .examples(Some(&named.profile_id))
            .await
            .unwrap()
            .len(),
        2
    );
    // Undoing one named line returns it to the model and takes its example with it.
    service.undo("s2").await.unwrap();
    assert_eq!(
        service
            .examples(Some(&named.profile_id))
            .await
            .unwrap()
            .len(),
        1
    );
    // Moving s1 away and back re-admits its example under the restored decision...
    service
        .correct(assign("s1", Some(&other.id), "assign"))
        .await
        .unwrap();
    service.undo("s1").await.unwrap();
    assert_eq!(
        service
            .examples(Some(&named.profile_id))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(service.examples(Some(&other.id)).await.unwrap().is_empty());
    // ...and undoing the name removes it as well.
    service.undo_identification(&named.id).await.unwrap();
    assert!(service.examples(None).await.unwrap().is_empty());
}
#[tokio::test]
async fn a_name_already_in_use_is_refused_for_a_new_person() {
    let (_tmp, _store, service) = fixture().await;
    let person = service.create("Person A").await.unwrap();
    assert!(service.create(" person a ").await.is_err());
    assert!(
        service
            .identify_cluster(identify(PersonChoice::New("PERSON A".into())))
            .await
            .is_err()
    );
    let other = service.create("Person B").await.unwrap();
    assert!(
        service
            .rename(parakeet_application::speakers::RenameSpeakerInput {
                id: other.id.clone(),
                name: "Person A".into(),
            })
            .await
            .is_err()
    );
    // Renaming someone to their own name, in another case, is fine.
    service
        .rename(parakeet_application::speakers::RenameSpeakerInput {
            id: person.id,
            name: "PERSON A".into(),
        })
        .await
        .unwrap();
    assert_eq!(service.profiles().await.unwrap().len(), 2);
}
