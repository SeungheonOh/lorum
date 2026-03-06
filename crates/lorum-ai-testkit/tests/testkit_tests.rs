use std::path::{Path, PathBuf};

use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantMessage, AssistantMessageEvent, ModelRef,
    StreamBoundaryEvent, StreamDoneEvent, StreamErrorEvent, StreamStartEvent, StreamTextDelta,
    StopReason, TokenUsage,
};
use lorum_ai_testkit::{
    SequenceError, StreamFixture, assert_expected_stop_reason, assert_valid_sequence,
    generate_snapshot, load_fixture, load_fixtures_from_dir, run_regression_suite, snapshot_hash,
};

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
    assert!(matches!(err, lorum_ai_testkit::FixtureError::NotFound { .. }));
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
