/// Trybuild fixture: verifies that `#[hw_oracle_test]` expands without errors.
///
/// The macro should produce:
///   - `add_oracle_inner() -> OracleCase`   (original body)
///   - `add_oracle_sim()`                    (`#[test]`, always compiled)
///   - `add_oracle_hw()`                     (`#[test] #[cfg(feature="hw-oracle")] #[ignore]`)
///   - `add_oracle_diff()`                   (`#[test] #[cfg(feature="hw-oracle")] #[ignore]`)
use labwired_hw_oracle::OracleCase;
use labwired_hw_oracle_macros::hw_oracle_test;

#[hw_oracle_test]
fn add_oracle() -> OracleCase {
    OracleCase::stub()
}

fn main() {}
