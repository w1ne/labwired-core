# Motor parity fixture diagnosis — 2026-11-12

## Decision

Approve the proposed one-file golden-fixture correction for owner application and normal native/app test validation. The old golden encodes the pre-R4 STM32 N-only gate bug. The independently calculated replacement agrees with the current engine at all unchanged native tolerances. No runtime or numerical threshold change is required.

## First divergence

The fixed ELF enables CCER=0x41 at cycle 40,171: channel 1 main only and channel 2 N only, with CCR1=700, CCR2=299, PSC=3, ARR=999, MOE=1. Integration begins at timer phase 86 CPU cycles, with CNT=21. N-only follows OCREF: both commanded terminals are active and phase A/B see +12/-12 V around the floating neutral. The 12.5 ns first step must therefore give currents +0.0008333333333333332/-0.0008333333333333332 A. Current WASM and the independent equations both give exactly those values.

The pre-R4 rule inverted N-only OCREF and applied symmetric half-dead-time offsets. It instead starts excitation at cycle 41,377. Independent replay of that old gate rule, using the identical first-100,000-cycle MMIO schedule, reproduces the old golden position (0.27596760305632023 versus 0.2759676030563203) and speed (3018.2824068036143 versus 3018.2824068036257). The corrected rule gives 0.2877425637693264 rad and 4247.631058080654 rpm, agreeing with current WASM 0.28774256376932644 rad and 4247.6310580806485 rpm.

The primary source is ST RM0351 Rev10 page 976, table 199. It specifies OCxN=OCxREF xor CCxNP for N-only; complementation and dead time apply when both channel outputs are enabled. The official PDF URL and verified rule are in review-receipt.json.

## Independent validation

capture-current.mjs reads the existing a0b0 WASM without modifying source or building. It records the 42 firmware timer-configuration changes, 281 pad edges (zero drops), and the five snapshots. independent-replay.py integrates only the captured MMIO input schedule. It does not use captured motor position, speed or current as integration inputs. It derives terminal voltages from the documented N-only rule and independently implements the documented floating-neutral winding, exact open-terminal decay, current-balance projection and semi-implicit shaft equations in Python.

All 40 numeric and 25 exact assertions of the proposed fixture pass against the current WASM trace at the original native tolerance, 1e-12*max(1,abs(expected)). The worst error consumes 7.682% of that tolerance. The inverter-fault current-decay and recovery assertions remain true. Existing Rust/WASM test code is unchanged.

The fixture service interval is one 80 MHz engine cycle (12.5 ns), below both the old electrical substep cap (128.571 us) and the new coupled cap (1.36579 us). The cap change therefore does not cause this failure. At 1, 2, 4 and 8 subdivisions per CPU cycle, the independent first-sample position is 0.2877425637693264, 0.28774134891788244, 0.28774074138385897 and 0.28774043758989953 rad. Consecutive error reductions are 1.99964 and 1.99982. Speed error reductions are 2.00004 and 2.00003. The same command schedule therefore exhibits the expected first-order convergence. Richardson estimates put the original-step position error at 2.430e-6 rad and speed error at 0.007811%, far below the change caused by the corrected gate mapping.

## Proposed patch and required owner validation

motor-parity-reference.patch changes only crates/wasm/tests/fixtures/motor-parity-native.json. All replacement numeric values come from the independent replay. The patch does not change the ELF, runtime model, test cases, test tolerance, solver budget, performance budget, CI graph or human-source policy.

The owner should apply this exact patch, run the unchanged native motor_states integration test and app motor-wasm-parity test, and retain this reference input/replay/receipt with the change. A full native or app test rerun has not been performed by this reviewer. The captured MMIO schedule comes from the real emulated firmware, so this validation is independent of the motor equations and gate evaluator, but does not claim an independent Cortex-M/controller implementation or physical hardware evidence.
