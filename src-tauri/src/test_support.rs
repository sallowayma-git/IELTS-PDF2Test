//! Shared helpers for tests that depend on the private, intentionally-uncommitted
//! regression corpus, plus the audio/listening fixtures several modules need to build a paper
//! that the packaging and probing paths can actually re-check against disk.
//!
//! `fixtures/golden/private-real/README.md` states the PDFs there are git-ignored because they
//! are private/copyrighted regression inputs. Tests that hard-failed on their absence made
//! `cargo test` permanently red on every clean checkout and in CI, which hides real regressions
//! in the noise instead of catching them. These helpers turn "corpus absent" into a visible skip
//! by default, and keep it a hard failure on machines that do mount the corpus.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Env var that turns a missing private fixture back into a hard failure.
pub(crate) const REQUIRE_PRIVATE_CORPUS_ENV: &str = "EPIC8_REQUIRE_PRIVATE_CORPUS";

/// Sample rate every generated listening fixture uses.
pub(crate) const FIXTURE_SAMPLE_RATE_HZ: u32 = 16_000;

/// A real, decodable 1 second 16 kHz mono 16-bit PCM WAV carrying a sine tone.
///
/// Listening tests used to declare a made-up `sha256` such as `"aaa…"`. That works for pure
/// schema validation, but every real path (NAS package staging, the student loader probe, the
/// managed-audio store) re-hashes the bytes it finds on disk, so a fake digest can only ever
/// produce a hash mismatch that looks like a product bug. Fixtures now derive the digest from
/// the bytes they actually stage, which is the same rule the product follows: the id of an
/// audio asset *is* its content hash.
pub(crate) fn wav_bytes_with_tone(hz: f64) -> Vec<u8> {
    let samples = (0..FIXTURE_SAMPLE_RATE_HZ)
        .map(|index| {
            ((index as f64 / FIXTURE_SAMPLE_RATE_HZ as f64) * hz * std::f64::consts::TAU).sin()
                * 8_000.0
        })
        .map(|value| value as i16)
        .collect::<Vec<_>>();
    let data = (samples.len() * 2) as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&FIXTURE_SAMPLE_RATE_HZ.to_le_bytes());
    bytes.extend_from_slice(&(FIXTURE_SAMPLE_RATE_HZ * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// The default fixture tone. Distinct parts get distinct tones so their asset ids differ the way
/// four real uploads would.
pub(crate) fn audio_bytes() -> Vec<u8> {
    wav_bytes_with_tone(440.0)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(crate) fn audio_sha256() -> String {
    sha256_hex(&audio_bytes())
}

/// A tone for part `ordinal` (1-based), returned as `(bytes, sha256)`.
///
/// Four parts need four *different* audio files; reusing one tone for every part would make the
/// fixture pass by accident while a real paper could never look like that.
pub(crate) fn part_audio(ordinal: u32) -> (Vec<u8>, String) {
    let bytes = wav_bytes_with_tone(220.0 * f64::from(ordinal.max(1)));
    let sha = sha256_hex(&bytes);
    (bytes, sha)
}

/// Write a tone to `path`, creating parent directories, and return the bytes written.
pub(crate) fn write_audio_fixture(path: &Path, hz: f64) -> Vec<u8> {
    let bytes = wav_bytes_with_tone(hz);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::File::create(path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    bytes
}

/// Audio descriptor for part `ordinal`. The digest is derived from the bytes
/// `stage_listening_audio` writes, so the packaging and probing paths can re-hash the file on
/// disk and agree with the descriptor — which is the rule the product follows.
pub(crate) fn listening_audio_asset(ordinal: u32) -> crate::schema::common::AssetDescriptorV2 {
    let (bytes, sha) = part_audio(ordinal);
    serde_json::from_value(serde_json::json!({
        "assetId": format!("audio-{sha}"),
        "kind": "audio",
        "mime": "audio/wav",
        "relativePath": format!("audio/{sha}.wav"),
        "sha256": sha,
        "byteLength": bytes.len(),
        "durationMs": 1000,
        "extractionMode": "user_upload"
    }))
    .expect("listening audio asset descriptor matches the schema")
}

/// Per-part `media` pointing at the same asset `listening_audio_asset` describes, with a probe
/// that passed (the compiler refuses a part whose audio was never decoded).
pub(crate) fn listening_audio_media(
    ordinal: u32,
) -> crate::schema::ielts_authoring_v2::ListeningPartMediaV2 {
    let (_, sha) = part_audio(ordinal);
    serde_json::from_value(serde_json::json!({
        "assetId": format!("audio-{sha}"),
        "mime": "audio/wav",
        "durationMs": 1000,
        "channels": 1,
        "sampleRateHz": FIXTURE_SAMPLE_RATE_HZ,
        "sha256": sha,
        "probe": {
            "status": "passed",
            "provider": "symphonia",
            "providerVersion": "0.6.0",
            "probedAt": "2026-09-22T00:00:00Z",
            "issueCodes": []
        }
    }))
    .expect("listening part media matches the schema")
}

/// One section of a synthetic listening paper.
pub(crate) struct ListeningPartSpec {
    pub(crate) ordinal: u32,
    pub(crate) questions: Vec<u32>,
}

impl ListeningPartSpec {
    /// A section holding the inclusive question range `start..=end`.
    pub(crate) fn range(ordinal: u32, start: u32, end: u32) -> Self {
        Self {
            ordinal,
            questions: (start..=end).collect(),
        }
    }
}

/// A complete-exam listening draft: one task group per section, every question scored, and each
/// section bound to **its own** audio asset.
///
/// Every section getting a distinct file matters. An earlier version of this fixture pointed all
/// four sections at one asset, which meant a packager that only staged a single file — or a
/// validator that only closed the asset set for one part — would still look correct. Four parts
/// with four digests is what a real paper looks like.
pub(crate) fn listening_exam(
    parts: Vec<ListeningPartSpec>,
) -> crate::schema::ielts_authoring_v2::IeltsAuthoringIRV2 {
    use crate::schema::ielts_authoring_v2::{
        AnswerSlotV2, AnswerValueV2, ExamModalityV2, IeltsAuthoringIRV2, ListeningPartV2,
        ListeningPlaybackPolicyV2, ListeningScopeV2, ListeningStructureV2, TaskGroupV2,
    };

    let mut task_groups = Vec::new();
    let mut answer_slots = BTreeMap::new();
    let mut answer_key = BTreeMap::new();
    let mut listening_parts = Vec::new();
    let mut assets = Vec::new();
    for spec in &parts {
        let mut slot_ids = Vec::new();
        for number in &spec.questions {
            let slot_id = format!("q{number}");
            slot_ids.push(slot_id.clone());
            answer_slots.insert(
                slot_id.clone(),
                serde_json::from_value::<AnswerSlotV2>(serde_json::json!({
                    "slotId": slot_id,
                    "questionNumber": number,
                    "displayLabel": number.to_string(),
                    "hostType": "paragraph",
                    "interaction": "text",
                    "participation": "scoring",
                    "constraints": {"maxWords": 1, "maxNumbers": 0},
                    "sourceAnchors": [],
                    "confidence": 1
                }))
                .unwrap(),
            );
            answer_key.insert(
                slot_id,
                AnswerValueV2::Text {
                    values: vec![format!("answer {number}")],
                    normalization: None,
                },
            );
        }
        let first = *spec.questions.first().expect("a section holds questions");
        let last = *spec.questions.last().expect("a section holds questions");
        task_groups.push(
            serde_json::from_value::<TaskGroupV2>(serde_json::json!({
                "taskId": format!("task-{}", spec.ordinal),
                "displayRange": {"kind": "range", "start": first, "end": last},
                "taskType": "note_completion",
                "instructions": [],
                "instructionSignature": {
                    "normalizedText": "Write ONE WORD ONLY.",
                    "taskType": "note_completion",
                    "expectedQuestionNumbers": spec.questions,
                    "expectedSlotCount": spec.questions.len(),
                    "selectionCardinality": {"min": 1, "max": 1, "exact": 1},
                    "answerAssignment": "per_slot",
                    "wordLimit": {"maxWords": 1, "maxNumbers": 0, "wordsAndOrNumber": false},
                    "evidenceAnchors": [],
                    "confidence": 1
                },
                "stimulus": [],
                "responseGroups": [{
                    "responseGroupId": format!("part-{}-response", spec.ordinal),
                    "kind": "text_entry",
                    "slotIds": slot_ids,
                    "cardinality": {"min": 1, "max": 1, "exact": 1},
                    "assignment": "per_slot",
                    "scoringPolicy": "per_slot_binary",
                    "duplicatePolicy": "ignore_duplicates",
                    "allowOptionReuse": false,
                    "sourceAnchors": []
                }],
                "sourceAnchors": [],
                "quality": {"score": 1, "sourceCoverage": 1, "hardFailures": []},
                "reviewState": "confirmed"
            }))
            .unwrap(),
        );
        listening_parts.push(ListeningPartV2 {
            part_id: format!("part-{}", spec.ordinal),
            display_label: format!("SECTION {}", spec.ordinal),
            expected_question_numbers: spec.questions.clone(),
            task_ids: vec![format!("task-{}", spec.ordinal)],
            cue: None,
            source_anchors: Vec::new(),
            media: Some(listening_audio_media(spec.ordinal)),
        });
        assets.push(listening_audio_asset(spec.ordinal));
    }

    // Start from the committed reading draft so every required piece of exam metadata is
    // genuinely schema-valid, then reshape the body into a listening paper. Hand-writing the
    // whole document here would let the fixture drift away from the schema unnoticed.
    let mut source: IeltsAuthoringIRV2 = serde_json::from_str(include_str!(
        "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
    ))
    .expect("committed authoring fixture parses");
    source.modality = ExamModalityV2::Listening;
    source.passage = None;
    source.assets = assets;
    source.listening = Some(ListeningStructureV2 {
        scope: ListeningScopeV2::CompleteExam,
        media: None,
        parts: listening_parts,
        playback_policy: serde_json::from_value::<ListeningPlaybackPolicyV2>(serde_json::json!({
            "mode": "practice",
            "autoplay": false,
            "allowPause": true,
            "allowSeek": true,
            "allowReplay": true,
            "refreshBehavior": "resume_from_snapshot",
            "crashRecoveryBehavior": "resume_from_snapshot",
            "showCurrentTime": true,
            "showDuration": true
        }))
        .unwrap(),
        transcript: None,
    });
    source.task_groups = task_groups;
    source.answer_slots = answer_slots;
    source.answer_key = answer_key;
    source.source_document_id = "doc-listening".to_string();
    source.audit.revision = 3;
    source
}

/// The standard four-section, 1–40 listening paper.
pub(crate) fn complete_listening_exam() -> crate::schema::ielts_authoring_v2::IeltsAuthoringIRV2 {
    listening_exam(vec![
        ListeningPartSpec::range(1, 1, 10),
        ListeningPartSpec::range(2, 11, 20),
        ListeningPartSpec::range(3, 21, 30),
        ListeningPartSpec::range(4, 31, 40),
    ])
}

/// Write each section's own audio under `asset_root/audio/<sha256>.wav` and return the relative
/// paths written, in ordinal order.
pub(crate) fn stage_listening_audio(asset_root: &Path, ordinals: &[u32]) -> Vec<String> {
    let mut written = Vec::new();
    for ordinal in ordinals {
        let (bytes, sha) = part_audio(*ordinal);
        let relative = format!("audio/{sha}.wav");
        let path = asset_root.join(&relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, &bytes).unwrap();
        written.push(relative);
    }
    written
}

/// Resolve a workspace-relative path (the crate lives in `src-tauri/`).
pub(crate) fn workspace_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn require_private_corpus() -> bool {
    std::env::var(REQUIRE_PRIVATE_CORPUS_ENV).is_ok_and(|value| value == "1")
}

/// True when every path exists. When some are missing this either panics (strict mode) or
/// prints a skip notice and returns false, so the caller can return early.
pub(crate) fn private_corpus_ready(test_name: &str, paths: &[PathBuf]) -> bool {
    let missing = paths
        .iter()
        .filter(|path| !path.exists())
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return true;
    }
    if require_private_corpus() {
        panic!(
            "{test_name}: {} required private fixture(s) missing: {}",
            missing.len(),
            missing.join(", ")
        );
    }
    eprintln!(
        "SKIP {test_name}: private regression corpus not present ({} missing, first: {}). Set {}=1 to make this a hard failure.",
        missing.len(),
        missing.first().map(String::as_str).unwrap_or("<none>"),
        REQUIRE_PRIVATE_CORPUS_ENV
    );
    false
}

/// Convenience form for a single workspace-relative fixture.
pub(crate) fn private_fixture_ready(test_name: &str, relative: &str) -> bool {
    private_corpus_ready(test_name, &[workspace_path(relative)])
}

/// True when the Python parser sidecar has the optional `pypdf` dependency available.
/// Image extraction tests skip without it rather than reporting a product failure.
pub(crate) fn python_pypdf_available() -> bool {
    for (command, args) in [("python", vec![]), ("python3", vec![]), ("py", vec!["-3"])] {
        let mut invocation = std::process::Command::new(command);
        invocation.args(args);
        invocation.args(["-c", "import pypdf"]);
        if let Ok(status) = invocation.status() {
            if status.success() {
                return true;
            }
        }
    }
    eprintln!("SKIP: python sidecar dependency pypdf is unavailable in this environment.");
    false
}

/// The authoritative eight-PDF private corpus selected by
/// `fixtures/golden/manifest.json#requiredPrivateCorpus`.
pub(crate) fn golden_private_corpus_paths() -> Vec<PathBuf> {
    let manifest_path = workspace_path("fixtures/golden/manifest.json");
    let Ok(bytes) = std::fs::read(&manifest_path) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    manifest
        .get("requiredPrivateCorpus")
        .and_then(serde_json::Value::as_array)
        .map(|fixtures| {
            fixtures
                .iter()
                .filter_map(|fixture| {
                    fixture
                        .get("sourcePath")
                        .and_then(serde_json::Value::as_str)
                })
                .map(workspace_path)
                .collect()
        })
        .unwrap_or_default()
}

/// Guard for the whole eight-PDF corpus. Returns false (after printing a skip notice) when any
/// selected fixture is absent, unless strict mode is requested.
pub(crate) fn golden_private_corpus_ready(test_name: &str) -> bool {
    let paths = golden_private_corpus_paths();
    if paths.is_empty() {
        if require_private_corpus() {
            panic!("{test_name}: golden manifest declares no requiredPrivateCorpus");
        }
        eprintln!("SKIP {test_name}: golden manifest declares no requiredPrivateCorpus.");
        return false;
    }
    private_corpus_ready(test_name, &paths)
}
