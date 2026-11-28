# SmartBasket routing review — 2026-11-12

This is a simulation-only source review and regression record. It is NOT a new
silicon capture, a hardware acceptance report, or a structural basket rating.
All `silicon.last_capture` values and historical capture results are unchanged.

## Scope of the content-bound drift acknowledgement

Compared with core main, the H563, L476, L073, F103 and F407 watched files change
only because `peripherals/gpio.rs` gains the `GpioRegisterBank` data type next
to the existing GPIO routing metadata. No STM32 register implementation, reset
value, pin mode, voltage threshold, or peripheral timing is edited. The new
peripheral capability defaults to an empty bank list.

The ESP32-S3 watched implementation adds explicit IN/OUT bank metadata, and its
chip descriptor gains datasheet-derived nominal IO levels. It does not change
the GPIO register reset values. The routing change fixes GPIO32–48 aliasing
and makes the resolved chip's IO metadata reach native, WASM and world
constructors. Table 5-4 of the ESP32-S3 Series Datasheet v2.2 supplies VIL <=
0.25*VDD and VIH >= 0.75*VDD. The test configuration assumes VDD = 3.3 V; it is
not a measurement of a physical board supply.

Only these six affected entries receive a new content-bound acknowledgement.
It expires on 2026-11-19. The generated status must continue to say re-capture
pending. No hardware verification date is advanced. This acknowledgement is
not evidence that the new routing or thresholds have been compared to live
silicon; that remains a separate acceptance step.

## Executed local checks

- Core unit suite: 5600 passed, zero failed, three ignored, including the
  unchanged downcast-count guard.
- Co-simulation routing: 29 passed. ESP32-S3 bank routing: two passed, covering
  boundary pins 0/31/32 and 8/9/40/48. HX711 parity: nine passed, including
  scalar attachment, signed values, fractional grams and mid-frame changes.
- IO-metadata construction regression: one test passed, exercising both S3
  profiles, ROM/fast boot, and present/absent metadata (eight combinations).
  Missing metadata remains a binding error rather than acquiring defaults.
- Native CLI/WASM host checks passed. All-target Clippy and the host-workspace
  check passed before this documentation-only acknowledgement update.
- Native S3 ROM-boot firmware replay: 8/8 assertions passed after 37,149,660
  primary steps. Cold boot with 700 g: 3/3 passed, no zeroed payload reading.
  Physics-derived 20-degree tilt inputs: 3/3 passed, no accepted weight reading.
- Web compiler regression subset: 157 passed; TypeScript check passed. API
  requirements regression: 15 passed. These are local integration results,
  not production deployment evidence.

The complete flash input SHA-256 is
`b4dccec5d9430d2c41c33aab92e122edc231b318898090cb65439c3deed4f580`.
The input manifest separately hashes the native executable and both ROM images.
The current ELF-less CLI result's `firmware_hash` is the empty-slice digest;
that known provenance gap must not be represented as a correct flash hash.

The basket dynamics use an assumed rigid support/force-sensor fixture. Source
Rev3 enclosure meshes are visual/fit inputs, not a validated load path. Android,
camera recognition, PCB production readiness and structural strength remain
outside this result. See issue #1846 and web PR #3172 for hosted integration.
