/// Trybuild snapshot tests for the `#[hw_oracle_test]` macro expansion.
///
/// `t.pass(...)` compiles the fixture and checks it produces no errors.
/// The compiled artefacts are cached; a snapshot of the stderr output is
/// stored under `tests/cases/*.stderr` when the test first runs.
#[test]
fn macro_expansion() {
    let t = trybuild::TestCases::new();
    t.pass("tests/cases/add_oracle.rs");
}
