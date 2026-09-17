use super::parse_size;

#[test]
fn bare_integers_are_byte_counts() {
    assert_eq!(parse_size("1048576").unwrap(), 1_048_576);
    assert_eq!(parse_size("262144").unwrap(), 262_144);
    assert_eq!(parse_size("  4096  ").unwrap(), 4096);
}

#[test]
fn unit_suffixes_still_parse() {
    assert_eq!(parse_size("512KB").unwrap(), 524_288);
    assert_eq!(parse_size("1564672B").unwrap(), 1_564_672);
    assert_eq!(parse_size("1KB").unwrap(), 1024);
}

#[test]
fn garbage_still_errors() {
    assert!(parse_size("not-a-size").is_err());
}
