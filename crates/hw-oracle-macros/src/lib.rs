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

/// RISC-V / ESP32-C3 sibling of [`hw_oracle_test`].
///
/// Expands an annotated function returning a `RiscVOracleCase` into three
/// variants:
///
/// * `<name>_sim`  — always compiled, runs against the software simulator.
/// * `<name>_hw`   — gated on `feature = "hw-oracle-c3"`; requires a
///   USB-JTAG-attached ESP32-C3 board.  Marked `#[ignore]`.
/// * `<name>_diff` — gated on `feature = "hw-oracle-c3"`; runs both and diffs.
///   Marked `#[ignore]`.
#[proc_macro_attribute]
pub fn riscv_oracle_test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let inner_name = format_ident!("{}_inner", fn_name);
    let sim_name = format_ident!("{}_sim", fn_name);
    let hw_name = format_ident!("{}_hw", fn_name);
    let diff_name = format_ident!("{}_diff", fn_name);

    let mut inner_fn = input.clone();
    inner_fn.sig.ident = inner_name.clone();

    let expanded = quote! {
        #inner_fn

        #[test]
        fn #sim_name() {
            labwired_hw_oracle::riscv::run_sim(#inner_name());
        }

        #[test]
        #[cfg(feature = "hw-oracle-c3")]
        #[ignore = "hw-oracle-c3: requires connected ESP32-C3 board"]
        fn #hw_name() {
            labwired_hw_oracle::riscv::run_hw(#inner_name());
        }

        #[test]
        #[cfg(feature = "hw-oracle-c3")]
        #[ignore = "hw-oracle-c3: requires connected ESP32-C3 board"]
        fn #diff_name() {
            labwired_hw_oracle::riscv::run_diff(#inner_name());
        }
    };

    TokenStream::from(expanded)
}

/// ARM Thumb / STM32 sibling of [`hw_oracle_test`].
///
/// Expands an annotated function returning a `ThumbOracleCase` into three
/// variants:
///
/// * `<name>_sim`  — always compiled, runs against the software simulator.
/// * `<name>_hw`   — gated on `feature = "hw-oracle-stm32"`; requires
///   an SWD-attached STM32 board.  Marked `#[ignore]`.
/// * `<name>_diff` — gated on `feature = "hw-oracle-stm32"`; runs both
///   and diffs.  Marked `#[ignore]`.
#[proc_macro_attribute]
pub fn thumb_oracle_test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let inner_name = format_ident!("{}_inner", fn_name);
    let sim_name = format_ident!("{}_sim", fn_name);
    let hw_name = format_ident!("{}_hw", fn_name);
    let diff_name = format_ident!("{}_diff", fn_name);

    let mut inner_fn = input.clone();
    inner_fn.sig.ident = inner_name.clone();

    let expanded = quote! {
        #inner_fn

        #[test]
        fn #sim_name() {
            labwired_hw_oracle::arm_thumb::run_sim(#inner_name());
        }

        #[test]
        #[cfg(feature = "hw-oracle-stm32")]
        #[ignore = "hw-oracle-stm32: requires connected STM32 board"]
        fn #hw_name() {
            labwired_hw_oracle::arm_thumb::run_hw(#inner_name());
        }

        #[test]
        #[cfg(feature = "hw-oracle-stm32")]
        #[ignore = "hw-oracle-stm32: requires connected STM32 board"]
        fn #diff_name() {
            labwired_hw_oracle::arm_thumb::run_diff(#inner_name());
        }
    };

    TokenStream::from(expanded)
}
