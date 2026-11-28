# HeyPCB INVADER portable Flex run

This fixture uses the existing generic Flex runtime for the imported
HeyPCB/led-art INVADER board:

- Source: HeyPCB/led-art at commit
  b0830dcfa3f71c409e9784fa784b9c6864a6ff12,
  boards/arcade-invader, CERN-OHL-S-2.0.
- Imported board: CH32V003F4P6, with the D1 21-pixel WS2812 strip on U1.PC6.
- Flex handoff: the named led_data port and the generic neopixel sink use the
  imported U1.PC6 data net. The Library lane owns discovery, import and browser
  launch; this fixture owns only the portable run contract and evidence.

The board source has no firmware and says it has not been fabricated,
assembled or powered. firmware.rs is newly authored portable WebAssembly
behavior. It sends one 21-pixel GRB frame: pixel 0 is 20 10 08, pixel 1 is
01 02 03, and the remaining pixels are dark. It names a port id only; no MCU
registers or target-specific code run.

test-positive.yaml requires the imported PC6 net to decode all 21 pixels and
both lit pixels. test-negative.yaml sends an all-dark frame on the same
imported PC6 net and requires 21 decoded pixels with none lit. Both scripts
assert the 1008 nanosecond-timestamped data edges. The runtime report pins the
manifest and firmware bytes and keeps portable firmware scope explicit.

The reusable WS2812 decoder observes the digital data waveform. This
functional slice does not calculate LED current, rail behavior, optical
brightness, USB-C behavior, MCU instruction or peripheral fidelity, or full
board coverage. It makes no chip-parity or physical-board claim. The board has
no analog sensor or analog output in this workload.

## Rebuild and run

    sh crates/wasm/tests/fixtures/flex-heypcb-led-art-arcade-invader/build.sh
    cargo test -p labwired-cli --test flex_test_runner flex_heypcb_led_art_invader

Run the browser Flex harness against the same fixture with
scripts/test_browser_flex.mjs when its Playwright module is available. The
fixture's `native.json` records the native CLI reference; `checks.mjs` runs
both scripts through the compiled `WasmFlexTestRun` API and compares the
report, pixel state, named port, and digital trace in Node or Chromium:

    node scripts/test_browser_flex.mjs \
      --fixture crates/wasm/tests/fixtures/flex-heypcb-led-art-arcade-invader \
      --check runLedArtChecks

For Chromium, build with `wasm-pack build crates/wasm --target web --dev
--out-dir pkg-flex-web` and set `LABWIRED_PLAYWRIGHT_MODULE` as described in
the script header.
