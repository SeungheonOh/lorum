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
