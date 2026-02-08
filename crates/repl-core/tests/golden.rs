mod common;
use repl_core::parse_policy;

#[test]
fn test_simple_policy() {
    let policy = common::read_file("tests/samples/simple_policy.policy");
    assert!(parse_policy(&policy).is_ok());
}
