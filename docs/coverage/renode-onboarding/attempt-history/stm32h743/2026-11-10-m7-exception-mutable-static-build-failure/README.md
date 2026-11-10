# STM32H743 Cortex-M7 exception smoke build failure

The target-only `stm32h743` inventory run stopped during `build_firmware`, before any firmware execution. Rust rejected `addr_of_mut!(RAM_VECTOR_TABLE.0)` because accessing a `static mut` requires an `unsafe` block. The scoreboard recorded this target as `fail` with `run_attempted: false`; no boot or smoke result was produced. The source access is wrapped in `unsafe` for the retry.
