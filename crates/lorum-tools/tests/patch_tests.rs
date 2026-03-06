use lorum_tools::internals::patch::{apply_hunk, parse_hunks, Hunk, HunkLine};

#[test]
fn parse_single_hunk() {
    let diff = "\
@@ some anchor
 context line
-old line
+new line
 more context";
    let hunks = parse_hunks(diff);
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].anchor.as_deref(), Some("some anchor"));
    assert_eq!(hunks[0].lines.len(), 4);
}

#[test]
fn parse_multiple_hunks() {
    let diff = "\
@@
 first
-remove
+add
@@ second anchor
 another
+inserted";
    let hunks = parse_hunks(diff);
    assert_eq!(hunks.len(), 2);
    assert!(hunks[0].anchor.is_none());
    assert_eq!(hunks[1].anchor.as_deref(), Some("second anchor"));
}

#[test]
fn apply_simple_replacement() {
    let file_lines: Vec<String> = vec![
        "line 1".into(),
        "line 2".into(),
        "line 3".into(),
    ];
    let hunk = Hunk {
        anchor: None,
        lines: vec![
            HunkLine::Context("line 1".into()),
            HunkLine::Remove("line 2".into()),
            HunkLine::Add("replaced line 2".into()),
            HunkLine::Context("line 3".into()),
        ],
    };
    let (result, _) = apply_hunk(&file_lines, &hunk, 0).unwrap();
    assert_eq!(result, vec!["line 1", "replaced line 2", "line 3"]);
}

#[test]
fn apply_with_anchor() {
    let file_lines: Vec<String> = vec![
        "fn foo() {".into(),
        "    let x = 1;".into(),
        "}".into(),
        "fn bar() {".into(),
        "    let y = 2;".into(),
        "}".into(),
    ];
    let hunk = Hunk {
        anchor: Some("fn bar".into()),
        lines: vec![
            HunkLine::Context("    let y = 2;".into()),
            HunkLine::Remove("}".into()),
            HunkLine::Add("    let z = 3;".into()),
            HunkLine::Add("}".into()),
        ],
    };
    let (result, _) = apply_hunk(&file_lines, &hunk, 0).unwrap();
    assert_eq!(
        result,
        vec![
            "fn foo() {",
            "    let x = 1;",
            "}",
            "fn bar() {",
            "    let y = 2;",
            "    let z = 3;",
            "}",
        ]
    );
}

#[test]
fn context_mismatch_returns_error() {
    let file_lines: Vec<String> = vec![
        "line 1".into(),
        "line 2".into(),
    ];
    let hunk = Hunk {
        anchor: None,
        lines: vec![
            HunkLine::Context("wrong context".into()),
            HunkLine::Remove("line 2".into()),
            HunkLine::Add("new".into()),
        ],
    };
    assert!(apply_hunk(&file_lines, &hunk, 0).is_err());
}
