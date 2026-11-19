# Attempt 1: circular DMA half-transfer smoke compile

Failure identity: imported target `stm32f103`, system
`configs/systems/onboarding/stm32f103.yaml`, firmware crate
`firmware-f103-i2c-demo` (`thumbv7m-none-eabi`). The attempt ran against the
working tree based on `da17e8a0d28c683a94dd21feddb392ec85bbaa38`; source
`crates/firmware-f103-i2c-demo/src/main.rs` had SHA-256
`993b42602b542cddb4868af3bfe0b063028747023dc9015e8ef6360c7a260d2d`.

The exact command was:

```sh
PATH=/usr/bin:/home/andrii/.cargo/bin:/usr/local/bin:/bin CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 /usr/bin/python3 scripts/run_renode_onboarding_inventory.py --only stm32f103
```

At 2026-11-18 18:57:50–18:57:59 UTC, the runner returned `fail` at
`build_firmware`. No firmware execution or boot evidence was produced. Rust
reported `E0428`: `NVIC_ICPR` was defined both at line 59 and the newly added
DMA constants at line 162. The existing module-level pointer already serves
this use; the second definition was accidental.

The metrics, summary, and complete build/run stage logs are retained beside
this note. The cheapest discriminating test is to remove the duplicate
constant and rerun the same one-target command: a successful firmware build
eliminates this compile cause, after which the smoke assertions distinguish
HTIF/HTIE behavior.

No simulation or hardware conclusion is drawn from this build failure.
