use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use lorum_ai_contract::{AssistantMessageEvent, StopReason};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamFixture {
    pub name: String,
    pub events: Vec<AssistantMessageEvent>,
    pub expected_stop_reason: Option<StopReason>,
}

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("fixture not found: {path}")]
    NotFound { path: String },
    #[error("failed to read fixture {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse fixture {path}: {source}")]
    Parse {
        path: String,
        source: serde_json::Error,
    },
    #[error("fixture path is invalid utf-8: {path:?}")]
    InvalidPath { path: PathBuf },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SequenceError {
    #[error("event sequence is empty")]
    Empty,
    #[error("event sequence must begin with start event")]
    MissingStart,
    #[error("event sequence numbers must be strictly increasing")]
    NonMonotonicSequence,
    #[error("event sequence numbers must not repeat")]
    DuplicateSequence,
    #[error("event sequence must contain exactly one terminal event")]
    MissingOrDuplicateTerminal,
    #[error("terminal event must be the final event")]
    TerminalNotLast,
    #[error("done event message id must match start message id")]
    MessageIdMismatch,
    #[error("delta event for unopened block: {0}")]
    DeltaWithoutStart(String),
    #[error("block end without matching start: {0}")]
    EndWithoutStart(String),
    #[error("block start repeated without end: {0}")]
    DuplicateBlockStart(String),
    #[error("unclosed blocks: {0}")]
    UnclosedBlocks(String),
    #[error("fixture expected stop reason mismatch")]
    StopReasonMismatch,
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("failed to serialize event at index {index}: {source}")]
    Serialize {
        index: usize,
        source: serde_json::Error,
    },
}

pub fn load_fixture(path: impl AsRef<Path>) -> Result<StreamFixture, FixtureError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(FixtureError::NotFound {
            path: path.display().to_string(),
        });
    }

    let content = fs::read_to_string(path).map_err(|source| FixtureError::Read {
        path: path.display().to_string(),
        source,
    })?;

    serde_json::from_str(&content).map_err(|source| FixtureError::Parse {
        path: path.display().to_string(),
        source,
    })
}

pub fn load_fixtures_from_dir(path: impl AsRef<Path>) -> Result<Vec<StreamFixture>, FixtureError> {
    let mut files: Vec<PathBuf> = fs::read_dir(path.as_ref())
        .map_err(|source| FixtureError::Read {
            path: path.as_ref().display().to_string(),
            source,
        })?
        .map(|entry| {
            entry
                .map(|v| v.path())
                .map_err(|source| FixtureError::Read {
                    path: path.as_ref().display().to_string(),
                    source,
                })
        })
        .collect::<Result<_, _>>()?;

    files.retain(|p| p.extension().is_some_and(|v| v == "json"));
    files.sort();

    files.into_iter().map(load_fixture).collect()
}

pub fn assert_valid_sequence(events: &[AssistantMessageEvent]) -> Result<(), SequenceError> {
    if events.is_empty() {
        return Err(SequenceError::Empty);
    }

    if !matches!(events.first(), Some(AssistantMessageEvent::Start(_))) {
        return Err(SequenceError::MissingStart);
    }

    assert_deterministic_ordering(events)?;

    let terminal_indexes: Vec<usize> = events
        .iter()
        .enumerate()
        .filter_map(|(idx, event)| event.is_terminal().then_some(idx))
        .collect();

    if terminal_indexes.len() != 1 {
        return Err(SequenceError::MissingOrDuplicateTerminal);
    }

    let terminal_idx = terminal_indexes[0];
    if terminal_idx != events.len() - 1 {
        return Err(SequenceError::TerminalNotLast);
    }

    validate_message_id(events)?;
    validate_block_lifecycle(events)?;

    Ok(())
}

pub fn assert_deterministic_ordering(
    events: &[AssistantMessageEvent],
) -> Result<(), SequenceError> {
    let mut seen = HashSet::new();
    let mut previous: Option<u64> = None;

    for event in events {
        let seq = event.sequence_no();
        if !seen.insert(seq) {
            return Err(SequenceError::DuplicateSequence);
        }

        if let Some(prev) = previous {
            if seq <= prev {
                return Err(SequenceError::NonMonotonicSequence);
            }
        }
        previous = Some(seq);
    }

    Ok(())
}

pub fn assert_expected_stop_reason(fixture: &StreamFixture) -> Result<(), SequenceError> {
    let Some(expected) = fixture.expected_stop_reason else {
        return Ok(());
    };

    let actual = fixture
        .events
        .last()
        .and_then(AssistantMessageEvent::stop_reason);
    if actual == Some(expected) {
        Ok(())
    } else {
        Err(SequenceError::StopReasonMismatch)
    }
}

pub fn generate_snapshot(events: &[AssistantMessageEvent]) -> Result<String, SnapshotError> {
    let mut lines = Vec::with_capacity(events.len());

    for (index, event) in events.iter().enumerate() {
        let line = serde_json::to_string(event)
            .map_err(|source| SnapshotError::Serialize { index, source })?;
        lines.push(line);
    }

    Ok(lines.join("\n"))
}

pub fn snapshot_hash(snapshot: &str) -> String {
    let digest = Sha256::digest(snapshot.as_bytes());
    hex::encode(digest)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureRegressionResult {
    pub fixture_name: String,
    pub passed: bool,
    pub snapshot_hash: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionReport {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<FixtureRegressionResult>,
}

pub fn run_regression_suite(fixtures: &[StreamFixture]) -> RegressionReport {
    let mut results = Vec::with_capacity(fixtures.len());

    for fixture in fixtures {
        let mut errors = Vec::new();

        if let Err(err) = assert_valid_sequence(&fixture.events) {
            errors.push(format!("sequence validation failed: {err}"));
        }

        if let Err(err) = assert_expected_stop_reason(fixture) {
            errors.push(format!("stop reason validation failed: {err}"));
        }

        let snapshot_hash_value = match generate_snapshot(&fixture.events) {
            Ok(snapshot) => {
                let hash_a = snapshot_hash(&snapshot);
                let hash_b = snapshot_hash(&snapshot);
                if hash_a != hash_b {
                    errors.push("snapshot hash is not deterministic".to_string());
                }
                Some(hash_a)
            }
            Err(err) => {
                errors.push(format!("snapshot generation failed: {err}"));
                None
            }
        };

        results.push(FixtureRegressionResult {
            fixture_name: fixture.name.clone(),
            passed: errors.is_empty(),
            snapshot_hash: snapshot_hash_value,
            errors,
        });
    }

    let total = results.len();
    let passed = results.iter().filter(|result| result.passed).count();
    let failed = total - passed;

    RegressionReport {
        total,
        passed,
        failed,
        results,
    }
}

fn validate_message_id(events: &[AssistantMessageEvent]) -> Result<(), SequenceError> {
    let start_message_id = match &events[0] {
        AssistantMessageEvent::Start(v) => &v.message_id,
        _ => return Err(SequenceError::MissingStart),
    };

    match events.last() {
        Some(AssistantMessageEvent::Done(v)) => {
            if &v.message.message_id != start_message_id {
                return Err(SequenceError::MessageIdMismatch);
            }
        }
        Some(AssistantMessageEvent::Error(_)) => {}
        _ => return Err(SequenceError::MissingOrDuplicateTerminal),
    }

    Ok(())
}

fn validate_block_lifecycle(events: &[AssistantMessageEvent]) -> Result<(), SequenceError> {
    let mut open_blocks: HashMap<String, &'static str> = HashMap::new();

    for event in events {
        match event {
            AssistantMessageEvent::TextStart(v) => {
                open_start(&mut open_blocks, &v.block_id, "text")?
            }
            AssistantMessageEvent::TextDelta(v) => {
                ensure_open(&open_blocks, &v.block_id, "text")?;
            }
            AssistantMessageEvent::TextEnd(v) => close_end(&mut open_blocks, &v.block_id, "text")?,
            AssistantMessageEvent::ThinkingStart(v) => {
                open_start(&mut open_blocks, &v.block_id, "thinking")?
            }
            AssistantMessageEvent::ThinkingDelta(v) => {
                ensure_open(&open_blocks, &v.block_id, "thinking")?;
            }
            AssistantMessageEvent::ThinkingEnd(v) => {
                close_end(&mut open_blocks, &v.block_id, "thinking")?
            }
            AssistantMessageEvent::ToolCallStart(v) => {
                open_start(&mut open_blocks, &v.block_id, "toolcall")?
            }
            AssistantMessageEvent::ToolCallDelta(v) => {
                ensure_open(&open_blocks, &v.block_id, "toolcall")?;
            }
            AssistantMessageEvent::ToolCallEnd(v) => {
                close_end(&mut open_blocks, &v.block_id, "toolcall")?
            }
            AssistantMessageEvent::Start(_)
            | AssistantMessageEvent::Done(_)
            | AssistantMessageEvent::Error(_) => {}
        }
    }

    if !open_blocks.is_empty() {
        let mut keys: Vec<String> = open_blocks.into_keys().collect();
        keys.sort();
        return Err(SequenceError::UnclosedBlocks(keys.join(",")));
    }

    Ok(())
}

fn open_start(
    open_blocks: &mut HashMap<String, &'static str>,
    block_id: &str,
    kind: &'static str,
) -> Result<(), SequenceError> {
    match open_blocks.get(block_id) {
        Some(_) => Err(SequenceError::DuplicateBlockStart(block_id.to_string())),
        None => {
            open_blocks.insert(block_id.to_string(), kind);
            Ok(())
        }
    }
}

fn ensure_open(
    open_blocks: &HashMap<String, &'static str>,
    block_id: &str,
    kind: &'static str,
) -> Result<(), SequenceError> {
    match open_blocks.get(block_id) {
        Some(existing) if *existing == kind => Ok(()),
        _ => Err(SequenceError::DeltaWithoutStart(block_id.to_string())),
    }
}

fn close_end(
    open_blocks: &mut HashMap<String, &'static str>,
    block_id: &str,
    kind: &'static str,
) -> Result<(), SequenceError> {
    match open_blocks.get(block_id) {
        Some(existing) if *existing == kind => {
            open_blocks.remove(block_id);
            Ok(())
        }
        _ => Err(SequenceError::EndWithoutStart(block_id.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use lorum_ai_contract::{
        ApiKind, AssistantContent, AssistantMessage, ModelRef, StreamBoundaryEvent,
        StreamDoneEvent, StreamErrorEvent, StreamStartEvent, StreamTextDelta, TokenUsage,
    };

    use super::*;

    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(format!("{name}.json"))
    }

    fn model_ref() -> ModelRef {
        ModelRef {
            provider: "openai".to_string(),
            api: ApiKind::OpenAiResponses,
            model: "gpt-5.2".to_string(),
        }
    }

    fn done_event(seq: u64, message_id: &str, reason: StopReason) -> AssistantMessageEvent {
        AssistantMessageEvent::Done(StreamDoneEvent {
            sequence_no: seq,
            message: AssistantMessage {
                message_id: message_id.to_string(),
                model: model_ref(),
                content: vec![AssistantContent::Text(lorum_ai_contract::TextContent {
                    text: "done".to_string(),
                })],
                usage: TokenUsage::default(),
                stop_reason: reason,
            },
        })
    }

    fn start_event(seq: u64, message_id: &str) -> AssistantMessageEvent {
        AssistantMessageEvent::Start(StreamStartEvent {
            sequence_no: seq,
            message_id: message_id.to_string(),
            model: model_ref(),
        })
    }

    #[test]
    fn load_fixture_reads_json_fixture() {
        let fixture = load_fixture(fixture_path("simple_text")).expect("load fixture");
        assert_eq!(fixture.name, "simple_text");
        assert!(!fixture.events.is_empty());
    }

    #[test]
    fn load_fixture_missing_path_fails() {
        let err = load_fixture(fixture_path("missing_fixture")).expect_err("must fail");
        assert!(matches!(err, FixtureError::NotFound { .. }));
    }

    #[test]
    fn load_fixtures_from_dir_sorts_by_filename() {
        let fixtures =
            load_fixtures_from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures"))
                .expect("load fixtures");
        let names: Vec<String> = fixtures.into_iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["error_stream", "simple_text", "tool_use"]);
    }

    #[test]
    fn valid_sequence_passes_for_simple_fixture() {
        let fixture = load_fixture(fixture_path("simple_text")).expect("load fixture");
        assert_valid_sequence(&fixture.events).expect("sequence should pass");
    }

    #[test]
    fn valid_sequence_passes_for_tool_use_fixture() {
        let fixture = load_fixture(fixture_path("tool_use")).expect("load fixture");
        assert_valid_sequence(&fixture.events).expect("sequence should pass");
    }

    #[test]
    fn empty_sequence_fails() {
        let err = assert_valid_sequence(&[]).expect_err("must fail");
        assert_eq!(err, SequenceError::Empty);
    }

    #[test]
    fn missing_start_fails() {
        let events = vec![done_event(1, "m1", StopReason::Stop)];
        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::MissingStart);
    }

    #[test]
    fn non_monotonic_sequence_fails() {
        let events = vec![
            start_event(2, "m1"),
            AssistantMessageEvent::Error(StreamErrorEvent {
                sequence_no: 1,
                code: "x".to_string(),
                message: "y".to_string(),
                retryable: false,
            }),
        ];

        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::NonMonotonicSequence);
    }

    #[test]
    fn duplicate_sequence_fails() {
        let events = vec![
            start_event(1, "m1"),
            AssistantMessageEvent::TextStart(StreamBoundaryEvent {
                sequence_no: 2,
                block_id: "b1".to_string(),
            }),
            AssistantMessageEvent::TextEnd(StreamBoundaryEvent {
                sequence_no: 2,
                block_id: "b1".to_string(),
            }),
            done_event(3, "m1", StopReason::Stop),
        ];

        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::DuplicateSequence);
    }

    #[test]
    fn missing_terminal_fails() {
        let events = vec![start_event(1, "m1")];
        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::MissingOrDuplicateTerminal);
    }

    #[test]
    fn duplicate_terminal_fails() {
        let events = vec![
            start_event(1, "m1"),
            done_event(2, "m1", StopReason::Stop),
            AssistantMessageEvent::Error(StreamErrorEvent {
                sequence_no: 3,
                code: "x".to_string(),
                message: "y".to_string(),
                retryable: false,
            }),
        ];
        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::MissingOrDuplicateTerminal);
    }

    #[test]
    fn terminal_not_last_fails() {
        let events = vec![
            start_event(1, "m1"),
            done_event(2, "m1", StopReason::Stop),
            AssistantMessageEvent::TextStart(StreamBoundaryEvent {
                sequence_no: 3,
                block_id: "b1".to_string(),
            }),
        ];

        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::TerminalNotLast);
    }

    #[test]
    fn done_message_id_mismatch_fails() {
        let events = vec![
            start_event(1, "m1"),
            done_event(2, "different", StopReason::Stop),
        ];

        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::MessageIdMismatch);
    }

    #[test]
    fn text_delta_without_start_fails() {
        let events = vec![
            start_event(1, "m1"),
            AssistantMessageEvent::TextDelta(StreamTextDelta {
                sequence_no: 2,
                block_id: "b1".to_string(),
                delta: "hello".to_string(),
            }),
            done_event(3, "m1", StopReason::Stop),
        ];

        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::DeltaWithoutStart("b1".to_string()));
    }

    #[test]
    fn block_end_without_start_fails() {
        let events = vec![
            start_event(1, "m1"),
            AssistantMessageEvent::TextEnd(StreamBoundaryEvent {
                sequence_no: 2,
                block_id: "b1".to_string(),
            }),
            done_event(3, "m1", StopReason::Stop),
        ];

        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::EndWithoutStart("b1".to_string()));
    }

    #[test]
    fn duplicate_block_start_fails() {
        let events = vec![
            start_event(1, "m1"),
            AssistantMessageEvent::TextStart(StreamBoundaryEvent {
                sequence_no: 2,
                block_id: "b1".to_string(),
            }),
            AssistantMessageEvent::TextStart(StreamBoundaryEvent {
                sequence_no: 3,
                block_id: "b1".to_string(),
            }),
            done_event(4, "m1", StopReason::Stop),
        ];

        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::DuplicateBlockStart("b1".to_string()));
    }

    #[test]
    fn unclosed_blocks_fail() {
        let events = vec![
            start_event(1, "m1"),
            AssistantMessageEvent::TextStart(StreamBoundaryEvent {
                sequence_no: 2,
                block_id: "b1".to_string(),
            }),
            done_event(3, "m1", StopReason::Stop),
        ];

        let err = assert_valid_sequence(&events).expect_err("must fail");
        assert_eq!(err, SequenceError::UnclosedBlocks("b1".to_string()));
    }

    #[test]
    fn expected_stop_reason_passes_when_matching() {
        let fixture = load_fixture(fixture_path("simple_text")).expect("load fixture");
        assert_expected_stop_reason(&fixture).expect("stop reason should match");
    }

    #[test]
    fn expected_stop_reason_fails_when_mismatch() {
        let mut fixture = load_fixture(fixture_path("simple_text")).expect("load fixture");
        fixture.expected_stop_reason = Some(StopReason::Length);

        let err = assert_expected_stop_reason(&fixture).expect_err("must fail");
        assert_eq!(err, SequenceError::StopReasonMismatch);
    }

    #[test]
    fn expected_stop_reason_no_expectation_is_noop() {
        let fixture = StreamFixture {
            name: "n/a".to_string(),
            events: vec![
                start_event(1, "m1"),
                AssistantMessageEvent::Error(StreamErrorEvent {
                    sequence_no: 2,
                    code: "transport".to_string(),
                    message: "broken".to_string(),
                    retryable: true,
                }),
            ],
            expected_stop_reason: None,
        };

        assert_expected_stop_reason(&fixture).expect("no expectation should pass");
    }

    #[test]
    fn snapshot_generation_is_stable_for_same_input() {
        let fixture = load_fixture(fixture_path("simple_text")).expect("load fixture");

        let snapshot_a = generate_snapshot(&fixture.events).expect("snapshot A");
        let snapshot_b = generate_snapshot(&fixture.events).expect("snapshot B");

        assert_eq!(snapshot_a, snapshot_b);
        assert_eq!(snapshot_hash(&snapshot_a), snapshot_hash(&snapshot_b));
    }

    #[test]
    fn snapshot_hash_changes_when_events_change() {
        let fixture = load_fixture(fixture_path("simple_text")).expect("load fixture");
        let mut mutated = fixture.events.clone();

        mutated.push(AssistantMessageEvent::Error(StreamErrorEvent {
            sequence_no: 99,
            code: "extra".to_string(),
            message: "mutated".to_string(),
            retryable: false,
        }));

        let baseline_hash = snapshot_hash(&generate_snapshot(&fixture.events).expect("snapshot"));
        let mutated_hash = snapshot_hash(&generate_snapshot(&mutated).expect("snapshot"));

        assert_ne!(baseline_hash, mutated_hash);
    }

    #[test]
    fn snapshot_contains_one_line_per_event() {
        let fixture = load_fixture(fixture_path("tool_use")).expect("load fixture");
        let snapshot = generate_snapshot(&fixture.events).expect("snapshot");
        let line_count = snapshot.lines().count();

        assert_eq!(line_count, fixture.events.len());
    }

    #[test]
    fn regression_suite_passes_for_baseline_fixtures() {
        let fixtures =
            load_fixtures_from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures"))
                .expect("load fixtures");
        let report = run_regression_suite(&fixtures);

        assert_eq!(report.total, 3);
        assert_eq!(report.failed, 0);
        assert_eq!(report.passed, 3);
        assert!(report
            .results
            .iter()
            .all(|result| result.passed && result.snapshot_hash.is_some()));
    }

    #[test]
    fn regression_suite_reports_invalid_fixture() {
        let bad_fixture = StreamFixture {
            name: "bad".to_string(),
            events: vec![AssistantMessageEvent::TextStart(StreamBoundaryEvent {
                sequence_no: 1,
                block_id: "b1".to_string(),
            })],
            expected_stop_reason: Some(StopReason::Stop),
        };

        let report = run_regression_suite(&[bad_fixture]);
        assert_eq!(report.total, 1);
        assert_eq!(report.failed, 1);
        assert!(!report.results[0].errors.is_empty());
    }
}
