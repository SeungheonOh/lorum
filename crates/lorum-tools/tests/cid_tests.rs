use lorum_tools::cid::{line_cid, parse_tag, validate_tag};

#[test]
fn cid_is_deterministic() {
    let a = line_cid(1, "hello world");
    let b = line_cid(1, "hello world");
    assert_eq!(a, b);
}

#[test]
fn cid_is_two_chars() {
    let cid = line_cid(42, "some content");
    assert_eq!(cid.len(), 2);
    assert!(cid.chars().all(|c| c.is_ascii_alphanumeric()));
}

#[test]
fn different_lines_produce_different_cids() {
    let a = line_cid(1, "line one");
    let b = line_cid(2, "line two");
    // Not guaranteed to differ for all inputs, but very likely
    // for these specific inputs
    assert_ne!(a, b);
}

#[test]
fn same_content_different_line_numbers_differ() {
    let a = line_cid(1, "same");
    let b = line_cid(2, "same");
    assert_ne!(a, b);
}

#[test]
fn parse_tag_valid() {
    let result = parse_tag("23#ZX");
    assert!(result.is_some());
    let (line, cid) = result.unwrap();
    assert_eq!(line, 23);
    assert_eq!(cid, "ZX");
}

#[test]
fn parse_tag_invalid_no_hash() {
    assert!(parse_tag("23ZX").is_none());
}

#[test]
fn parse_tag_invalid_cid_length() {
    assert!(parse_tag("23#Z").is_none());
    assert!(parse_tag("23#ZXY").is_none());
}

#[test]
fn parse_tag_invalid_line_number() {
    assert!(parse_tag("abc#ZX").is_none());
}

#[test]
fn validate_tag_works() {
    let lines = vec!["first line", "second line", "third line"];
    let cid = line_cid(2, "second line");
    let tag = format!("2#{cid}");
    let result = validate_tag(&tag, &lines);
    assert_eq!(result, Some(1)); // 0-based index
}

#[test]
fn validate_tag_wrong_cid() {
    let lines = vec!["first line", "second line"];
    let result = validate_tag("2#XX", &lines);
    // XX is almost certainly not the right CID
    // (could theoretically collide, but extremely unlikely)
    assert!(result.is_none());
}

#[test]
fn validate_tag_out_of_bounds() {
    let lines = vec!["only line"];
    let cid = line_cid(1, "only line");
    let tag = format!("5#{cid}");
    assert!(validate_tag(&tag, &lines).is_none());
}

#[test]
fn validate_tag_zero_line() {
    let lines = vec!["only line"];
    assert!(validate_tag("0#XX", &lines).is_none());
}
