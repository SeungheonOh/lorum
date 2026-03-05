/// Content ID (CID) generation for line-level addressing.
///
/// Each line gets a short 2-character alphanumeric tag derived from a hash of
/// the line content and its 1-based line number. The tag is deterministic:
/// the same (line_number, content) pair always produces the same CID.
///
/// Format in read output: `LINE#ID\tcontent`
/// Example: `23#ZX\t  const timeout = 30_000;`
const ALPHABET: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Generate a 2-character CID for a line.
///
/// Uses FNV-1a hash of (line_number, content) to produce a deterministic
/// 2-char code from [0-9A-Z].
pub fn line_cid(line_number: usize, content: &str) -> String {
    let hash = fnv1a(line_number, content);
    let a = ALPHABET[(hash % 36) as usize];
    let b = ALPHABET[((hash / 36) % 36) as usize];
    String::from_utf8(vec![a, b]).unwrap()
}

/// Parse a `LINE#ID` tag, returning `(line_number, cid)`.
///
/// Returns `None` if the tag doesn't match the expected format.
pub fn parse_tag(tag: &str) -> Option<(usize, String)> {
    let hash_pos = tag.find('#')?;
    let line_str = &tag[..hash_pos];
    let cid = &tag[hash_pos + 1..];
    if cid.len() != 2 {
        return None;
    }
    let line_number: usize = line_str.parse().ok()?;
    Some((line_number, cid.to_string()))
}

/// Validate that a tag matches the given file content.
///
/// Returns `Some(line_index)` (0-based) if the tag is valid for the given lines,
/// or `None` if the CID doesn't match.
pub fn validate_tag(tag: &str, lines: &[&str]) -> Option<usize> {
    let (line_number, cid) = parse_tag(tag)?;
    if line_number == 0 || line_number > lines.len() {
        return None;
    }
    let idx = line_number - 1;
    let expected = line_cid(line_number, lines[idx]);
    if cid == expected {
        Some(idx)
    } else {
        None
    }
}

fn fnv1a(line_number: usize, content: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    // Mix in line number bytes
    for byte in line_number.to_le_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    // Mix in content bytes
    for byte in content.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
