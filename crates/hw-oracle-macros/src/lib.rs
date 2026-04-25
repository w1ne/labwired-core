//! `#[hw_oracle_test]` procedural macro.
//!
//! Expands a single annotated function into three test variants:
//!
//! * `<name>_sim`  — always compiled, runs against the software simulator.
//! * `<name>_hw`   — gated on `feature = "hw-oracle"`, runs against physical
//!   hardware; marked `#[ignore]` so CI skips it unless the self-hosted
//!   runner explicitly passes `--ignored`.
//! * `<name>_diff` — gated on `feature = "hw-oracle"`, runs both and diffs
//!   the results; also `#[ignore]` for the same reason.
//!
//! The original function body is moved into `<name>_inner`, which returns the
//! `OracleCase` used by all three runners.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, ItemFn};

#[proc_macro_attribute]
pub fn hw_oracle_test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let inner_name = format_ident!("{}_inner", fn_name);
    let sim_name = format_ident!("{}_sim", fn_name);
    let hw_name = format_ident!("{}_hw", fn_name);
    let diff_name = format_ident!("{}_diff", fn_name);

    // Rename the original function to `<name>_inner`.
    let mut inner_fn = input.clone();
    inner_fn.sig.ident = inner_name.clone();

    let expanded = quote! {
        #inner_fn

        #[test]
        fn #sim_name() {
            labwired_hw_oracle::run_sim(#inner_name());
        }

        #[test]
        #[cfg(feature = "hw-oracle")]
        #[ignore = "hw-oracle: requires connected ESP32-S3 board"]
        fn #hw_name() {
            labwired_hw_oracle::run_hw(#inner_name());
        }

        #[test]
        #[cfg(feature = "hw-oracle")]
        #[ignore = "hw-oracle: requires connected ESP32-S3 board"]
        fn #diff_name() {
            labwired_hw_oracle::run_diff(#inner_name());
        }
    };

    TokenStream::from(expanded)
}
