# Next public mergeback: sensor DRDY and ESP32-S3 RGB/touch

## Decision and evidence boundary

Prepare two independent tranches. First port the generic retained open-drain resolver and opt-in autonomous data-ready events, then apply the bounded LSM303AGR descriptors. Prepare RGB565/GT911 as a separate functional-model change. Integrate the output resolver with the existing open device-output PR before publishing it. RGB changes require an explicit validation-drift acknowledgement decision because they touch the S3 silicon watch.

This map compares upstream [3e227a9b3b8332b38d33a721d2662b237a185ebb](https://github.com/w1ne/labwired-core/commit/3e227a9b3b8332b38d33a721d2662b237a185ebb) with public fork [d543a46bba894f9e7e3aa31cdba6feb561b2d08e](https://github.com/CrispStrobe/labwired-core/commit/d543a46bba894f9e7e3aa31cdba6feb561b2d08e). Findings below come from local source/diff inspection on 2026-11-10. Test names and commands describe available tests and required acceptance; this review did not run them. Fork documentation and commit messages are not independent test or silicon evidence.

Port selected hunks against current upstream. The fork rebased/reconciled its history: broad commits with unrelated subjects contain necessary registration and initialization lines. Whole-commit cherry-picks would also import unrelated models, receipts, performance changes or old examples. Keep the established release/CI lanes and performance thresholds. Keep user examples in the separate examples repository; use core test fixtures or an explicit pinned examples input for guest regression sources.

## 1. Retained open-drain outputs

### Source and extraction order

| Step | Exact public source commits | Required scope |
| --- | --- | --- |
| Resolver and retained pad state | [ac659c89](https://github.com/CrispStrobe/labwired-core/commit/ac659c8957bdca026fb9e0b58537a98df28aee3d), [7cf71c2f](https://github.com/CrispStrobe/labwired-core/commit/7cf71c2f05ae747c971ad411c71237a76d818e0f) | New `crates/core/src/bus/open_drain.rs`; named drivers keyed by device/role and sampled GPIO address/bit. |
| Attachment, resolution and clock-gated restoration | [d85eef69](https://github.com/CrispStrobe/labwired-core/commit/d85eef695cb53dabd7a61900ab4fdac43124ad51), [30b8162f](https://github.com/CrispStrobe/labwired-core/commit/30b8162f3c3a7bc3bcccdf77e006ee837a37e945), [ffd79fa2](https://github.com/CrispStrobe/labwired-core/commit/ffd79fa2b2738e9508fc078e7d8188e31df136b5) | `bus/device_hooks.rs`, `peripherals/kit/ctx.rs`, resolver and `tests/tier2_device_pins.rs`. Explicit `open_drain_pullups` placement option. |
| Explicit collector inversion and configuration rejection | [e1421d48](https://github.com/CrispStrobe/labwired-core/commit/e1421d48a038ab05c2a83bd132e92596d8d3c110), [d0e46c74](https://github.com/CrispStrobe/labwired-core/commit/d0e46c7445e28ad4c3fc89b7ba52627a4a03cee4), [ff5af144](https://github.com/CrispStrobe/labwired-core/commit/ff5af1442719c50fdf5c3b5d640505ede560db54) | Same production files; `inverting_collectors` is explicitly opt-in. Preserve malformed-pad, duplicate/unknown-role and mixed-mode rejection. |
| Required initialization hunks hidden in broad commits | [4cdd97db](https://github.com/CrispStrobe/labwired-core/commit/4cdd97dbb381e1cc52da1fa69863cb741492e985), [952fa635](https://github.com/CrispStrobe/labwired-core/commit/952fa6356566027392f3f5ad69c8e514793e7b4b) | Only `pub mod open_drain`, `SystemBus.device_open_drain_nets`, and its default initialization in `bus/construct.rs`, `bus/from_config.rs`, `bus/tests_main.rs`. Do not take the surrounding SAMD51/LEDC work. |

**Already upstream:** declarative output roles, output-pin binding, queued I2C device drives, GPIO sampling seams, timers/rule execution, and GPIO observers. **Missing:** retained multi-driver resolution, per-pad pull-up declaration and explicit collector inversion. This is distinct from existing MCU GPIO open-drain output semantics.

**Current source contract:** a registered driver starts released. A false level pulls low; a true level releases. All drivers must release before the pad reads high. Collector inversion maps a high source to a low shared line. A missing binding remains disconnected. A configured malformed binding is rejected. The resolver does not model floating voltage, propagation, push-pull contention or arbitrary circuit nets. Source: [resolver](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/crates/core/src/bus/open_drain.rs), [attachment](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/crates/core/src/peripherals/kit/ctx.rs).

### Acceptance and integration requirements

- Exercise two independent sinks on one real GPIO: both released, A low, B low, release A while B remains low, then release B. Check the sampled pad and edge observation.
- Verify retained high/low state survives initial attachment while the GPIO clock is gated and becomes visible when it opens.
- Test direct plus inverted drivers; duplicate/unknown roles; inverted roles not declared open drain; mixed legacy/open-drain attachment in both orders; malformed present pad; and valid unconnected role.
- Preserve a cheap empty fast path. Measure the bound path with its retained-net service enabled; do not adopt unrelated fork performance rebaselines.
- Run `tier2_device_pins` plus the resolver unit tests. Use the fork's [shared-line guest](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/crates/core/tests/microbit_v2_open_drain_guest.rs) as the guest-level reference. Its current build assumes `arm-none-eabi-gcc` and assembly under `examples/microbit-v2`; resolve that fixture dependency without recreating the examples tree in core.

Guest provenance includes [0f0a3ee9](https://github.com/CrispStrobe/labwired-core/commit/0f0a3ee90fc6b004a1bd14df9aab0ea44e2b1559), [b7156579](https://github.com/CrispStrobe/labwired-core/commit/b7156579dca6e7bea1c191a28179da89e8dd82df), [26cad6d3](https://github.com/CrispStrobe/labwired-core/commit/26cad6d34bc264eaf55af0c0e6adaca5992b722b), [f4be2b08](https://github.com/CrispStrobe/labwired-core/commit/f4be2b08b028343fabdcac025070ae630d5bd5f5), [5b87b328](https://github.com/CrispStrobe/labwired-core/commit/5b87b32846306ee2fe29222c1d90b72c77ad8566) and [68aa1c03](https://github.com/CrispStrobe/labwired-core/commit/68aa1c0381cfaf6273527c1ffa5ec5f4217e554b). Select the final fixture behavior, including real TWIM transactions and absent-address rejection.

## 2. Autonomous data-ready and LSM303AGR

### Source and scope

| Layer | Exact public sources | Port scope |
| --- | --- | --- |
| Generic event | [91935e5c](https://github.com/CrispStrobe/labwired-core/commit/91935e5ced58f293517b79ced4fd0c43d993a388) | `crates/config/src/rules.rs`; `components/declarative_i2c.rs`, `declarative_gpio.rs`, `declarative_uart.rs`, `rule_machine.rs`. Add `Event::DataReady`, serialization, declared-name validation and opted-in device-clock completion. |
| Cancellation and deterministic ordering | [bfc30fb5](https://github.com/CrispStrobe/labwired-core/commit/bfc30fb59216ebf98d2620321f6d3adaf8c519f1), [097527df](https://github.com/CrispStrobe/labwired-core/commit/097527df6359753568d40c5ec997a3a8d3ab12a1) | `declarative_i2c.rs`; preserve legacy unsubscribed read-driven promotion. |
| Required validation hardening | [c31cd9c5](https://github.com/CrispStrobe/labwired-core/commit/c31cd9c52bf12b64d19890280fbdf9753ebfee97), [16c46a4e](https://github.com/CrispStrobe/labwired-core/commit/16c46a4ebc2fff84f2a24283b56409f36148d07f), [708af5f8](https://github.com/CrispStrobe/labwired-core/commit/708af5f8735a336e1875c4f0c9327d1f465ed4cd), [b190d804](https://github.com/CrispStrobe/labwired-core/commit/b190d804368cda3652433b4eb6e392dd4c27b9c9) | Reject ambiguous zero-time conversions and rule/FIFO/implicit writes to subscribed start controls. These are part of the feature, not optional cleanup. |
| Sensor descriptors and acknowledgement groups | [e8e760bb](https://github.com/CrispStrobe/labwired-core/commit/e8e760bb1acf9dafa0340759af5561a80f98dfdd), [1870ba54](https://github.com/CrispStrobe/labwired-core/commit/1870ba5412b813d218dbbe1a854838f51c369a7f), [8c820d92](https://github.com/CrispStrobe/labwired-core/commit/8c820d9226d84b799a2b13899ba5796abbe6ca34) | `crates/config/src/peripherals.rs` adds `DataReadyRead`/`clear_on_read_all`; core acknowledgement state; `configs/devices/lsm303agr_{accel,mag}.yaml`; micro:bit placement and guest fixtures. |
| Scheduler-profile acceptance | [8f2d638c](https://github.com/CrispStrobe/labwired-core/commit/8f2d638c309b1865ef75a693b2ba05b2d71becb4) | Keep source-level scheduler-on/off guards and guest intervals. Extract relevant tests only. |

**Already upstream:** both LSM303AGR YAML devices, conversion readiness overlays, explicit simulation-time advancement, read acknowledgement, output rules and the micro:bit TWIM path. The old fork Rust LSM303AGR implementation is superseded; do not port it.

**Current source contract:** only subscribed conversions complete autonomously. The device orders multiple elapsed deadlines deterministically and fires once per conversion. Stopping an in-flight conversion cancels it; an already-ready result remains until acknowledged. The implementation rejects a descriptor that combines data-ready subscriptions with device timers. It also rejects other writers that bypass the subscribed start register's lifecycle, while preserving self-clearing GO-command behavior. Source: [current engine](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/crates/core/src/peripherals/components/declarative_i2c.rs).

The accelerometer requires every enabled output high byte, in any order, before group acknowledgement. The magnetometer accepts any output high byte. Neither a low byte nor a status read is an acknowledgement. The board configuration explicitly inverts the native sensor outputs into the shared P0.25 pulled-up line. The KL27 participant is absent.

**Limit claims to the source:** timing remains 10 ms regardless of selected ODR; samples remain live rather than BDU-latched; overrun/FIFO/threshold behavior is absent. Magnetometer qualification is bounded to continuous 100 Hz with threshold routing off. Idle reconfiguration retention is an internal consistency policy, not demonstrated silicon behavior. References: [accelerometer descriptor](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/configs/devices/lsm303agr_accel.yaml), [magnetometer descriptor](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/configs/devices/lsm303agr_mag.yaml).

### Acceptance

- No register polling: advance time to just before and at the conversion deadline; require the pin edge exactly once. Stop/rearm before deadline and ensure no stale completion.
- Advance across two deadlines in one call and verify deterministic ordering. Require load errors for timer combinations, missing/duplicate conversion names, zero duration and lifecycle-bypassing writers.
- Check legacy descriptors without subscriptions retain prior behavior.
- Accelerometer: enabled-axis subsets, arbitrary high-byte order, low-byte/status non-acknowledgement, repeated reads, no enabled axes, route/polarity changes, enable-after-ready, active rearm and idle retention. Check ready bits and IRQ agree.
- Magnetometer: high-byte-only acknowledgement, route masking/reenabling, cancellation and ready retention.
- Run the focused config/engine tests and `microbit_v2_mag_drdy_guest`, `microbit_v2_sensor_drdy_guest`, `microbit_v2_open_drain_guest` with and without `event-scheduler`, using the fixture's declared tick intervals. Add a negative control with autonomous completion disabled so guest assertions cannot pass through polling alone.

## 3. ESP32-S3 RGB565, ST7701 and GT911

**Main source:** [184359726288d048ef315d4fa4e99f1f737208a0](https://github.com/CrispStrobe/labwired-core/commit/184359726288d048ef315d4fa4e99f1f737208a0). Its 16-file diff adds 2,226 lines and removes 19; it combines engine, board, fixture and optional browser-input work. Split the port by dependency.

| Slice | Actual production scope and dependencies |
| --- | --- |
| RGB transport | `esp32s3/lcd_cam.rs`, `esp32s3/gdma.rs`, new `components/rgb_panel.rs`; add the `Peripheral::attach_rgb_panel` capability from [b627fafc](https://github.com/CrispStrobe/labwired-core/commit/b627fafc4e7bd9d853708b1a62495d7a32fe963b). |
| Panel and touch | New `components/st7701.rs` and `components/gt911.rs`. Extract module exports and only these two kit registrations from broad [0efad509](https://github.com/CrispStrobe/labwired-core/commit/0efad50949f8ab6c8e008d07b913ad363fd5658c). Do not import its unrelated ST7735/legacy sensor registrations. |
| Board and regression fixtures | `configs/systems/esp32-4848s040.yaml`, `crates/core/tests/esp32s3_rgb_s040.rs`, `tests/fixtures/esp32-4848s040/{build_probe.py,probe.elf,check_wasm.cjs,README.md}`, bounded board documentation. Adapt semihost smoke/fixture registration only if needed for the existing test lane. |
| Optional browser pad stimulus | The same source commit adds `wasm/src/inputs.rs` `set_gpio_external` and `wasm/src/lab_tools.rs` journal/replay support. Pair with the S3 unbonded-pad guard from [952fa635](https://github.com/CrispStrobe/labwired-core/commit/952fa6356566027392f3f5ad69c8e514793e7b4b). This is a separate generic API; touch uses the already-existing `set_inputs` interface. |

**Already upstream:** S3 CPU/boot, GPIO bank 1, PadPort external hold/release, I2C controller, LCD_CAM i80, GDMA, scheduler hooks, octal PSRAM/MMU, GPIO observers, component inputs and framebuffer artifacts. **Absent at the pinned base:** RGB frame receiver, RGB cyclic descriptor walker, ST7701, GT911 and RGB attachment capability. Bank-1 support is not a new fork feature; rejecting physical GPIO22–25 holes is a remaining fork delta.

**Current source behavior:** the RGB walker reads guest descriptors and data through the bus, rejects invalid/unaligned/CPU-owned descriptors and failed reads, observes ownership writeback, handles EOF within circular chains, and caps a burst at the active frame boundary. Odd descriptor splits preserve half pixels. LCD completion queues its existing scheduled event after the temporarily borrowed LCD model is restored. i80 retains its existing transfer path. ST7701 consumes GPIO 9-bit control edges and emits a 480×480 `rgb565_be` artifact with display/sleep/backlight state. GT911 keeps a report stable once observed until zero-write acknowledgement; later movement coalesces into the next report; held-contact refresh uses simulation time. See [LCD_CAM](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/crates/core/src/peripherals/esp32s3/lcd_cam.rs), [GDMA](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/crates/core/src/peripherals/esp32s3/gdma.rs), [GT911](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/crates/core/src/peripherals/components/gt911.rs).

**Do not promote fork claims beyond this boundary:** VSYNC is a completed DMA active-area event, not calibrated PCLK/porch timing. RGB pin mux and waveform generation are absent. The GT911 I2C route is metadata, not a wrong-pin NACK check. Touch is one contact with a modeled 10 ms period; reset/address strapping, gestures and EEPROM persistence are absent. The committed original bare ELF does not qualify arbitrary Arduino/LVGL firmware. The fork's [board guide](https://github.com/CrispStrobe/labwired-core/blob/d543a46bba894f9e7e3aa31cdba6feb561b2d08e/docs/boards/esp32-4848s040.md) itself records no physical S040 validation.

### Acceptance

- Rebuild the original fixture from its Python generator and compare its artifact hash. Do not substitute host drawing hooks.
- Run `esp32s3_rgb_s040` together with unchanged `esp32s3_lcd_i80_pixels`, both default and `--features event-scheduler`.
- Require asymmetric corner/stride checks, odd descriptor boundaries, byte/bit order, finite completion and W1C VSYNC, cyclic memory resampling, reset/stop with a partial frame, real mapped-PSRAM reads, and owner/writeback behavior.
- Missing, short, inaccessible, malformed, CPU-owned and unsupported-format inputs must not manufacture a completed frame. Ensure failures surface as appropriate descriptor errors.
- Exercise ST7701 sleep/display/reset/backlight, GT911 status/coordinate/ack coherence, and an actual guest down/move/up redraw. Run the GT911 unit tests and the Node WASM fixture when shipping a browser bundle.
- If extracting the pad API, validate finite integer pins before narrowing, reject 22–25 and out-of-range pins, test bank-0/bank-1 independence, hold/release, failed-call non-journaling and replay equivalence.
- Test negative controls with RGB completion disabled and touch state fixed. The acceptance suite must detect missing model behavior, not merely firmware progress.

## 4. Open-PR overlap and publication gates

The cached 2026-11-10 open-PR list contains 51 entries. No title specifically names these sensor/RGB features. This is not proof of no code overlap. The following overlaps were checked from locally available commits; branch heads may advance.

| Open PR and locally inspected head | Concrete consequence |
| --- | --- |
| [#1727](https://github.com/w1ne/labwired-core/pull/1727), [946a5a5a8e558060440e4b4cb6164a2303a1ae94](https://github.com/w1ne/labwired-core/commit/946a5a5a8e558060440e4b4cb6164a2303a1ae94) | Direct semantic overlap. It adds device-to-device observers and all-pad fanout in `device_hooks`, plus bus fields/constructors, `kit/ctx` and `tier2_device_pins`. The fork still selects the first matching pad and rejects repeated device/role bindings globally. Preserve #1727's observer-only bus servicing and fanout; apply retained resolution independently per destination pad. Specify raw source-observer versus resolved-pad semantics and test both. |
| [#1695](https://github.com/w1ne/labwired-core/pull/1695), [c1f4bcc7778688248cc8657305adf7e7b203019f](https://github.com/w1ne/labwired-core/commit/c1f4bcc7778688248cc8657305adf7e7b203019f) | Confirmed shared `device_hooks`, bus constructors/config, `Peripheral`, component exports and kit registry. Reconcile its timing/control foundation rather than replacing shared files. |
| [#1681](https://github.com/w1ne/labwired-core/pull/1681), [c72f25baedf3710152af3713f86b12d3ca841bcd](https://github.com/w1ne/labwired-core/commit/c72f25baedf3710152af3713f86b12d3ca841bcd) | Confirmed overlaps in declarative I2C/config, output binding, bus setup and kit registry. Local branch differs from the older 100-file API cache; use the exact local commit noted here. |
| [#1663](https://github.com/w1ne/labwired-core/pull/1663), [0e53269478d47c4d85bbe30826bef580dc31389d](https://github.com/w1ne/labwired-core/commit/0e53269478d47c4d85bbe30826bef580dc31389d) | Confirmed declarative I2C/config and bus/Peripheral overlaps. Its broad platform work is not a prerequisite for these ports. |
| [#1802](https://github.com/w1ne/labwired-core/pull/1802), [5ebf4b591a77be1faa3e32f6e3417bc409921bb7](https://github.com/w1ne/labwired-core/commit/5ebf4b591a77be1faa3e32f6e3417bc409921bb7); [#1799](https://github.com/w1ne/labwired-core/pull/1799), [116391f3c6e278027d135f1d0e81c6cc7084ee4c](https://github.com/w1ne/labwired-core/commit/116391f3c6e278027d135f1d0e81c6cc7084ee4c) | Nearby S3 display/ADC work, but no source-path overlap found in this map's bus/component/LCD_CAM/GDMA/kit scope at these local heads. Recheck when rebasing. |
| [#1739](https://github.com/w1ne/labwired-core/pull/1739) | SAMD21 platform work may touch common paths. Its head was not present locally in this review; no exact code-overlap claim. |

The pinned [validation manifest](https://github.com/w1ne/labwired-core/blob/3e227a9b3b8332b38d33a721d2662b237a185ebb/validation/manifest.yaml) watches exact files and directory prefixes. For the proposed generic resolver/DRDY production paths, no silicon-backed watch overlaps were found; `bus/from_config.rs` participates in 22 simulation-derived records. RGB's `esp32s3/lcd_cam.rs`, `gdma.rs` and optional `gpio.rs` all match the silicon-backed `esp32s3` directory watch, plus the simulation-only S3-Zero record. `Peripheral` also participates in three simulation-derived records.

Regenerate simulation-derived records through the existing tooling. Keep all existing drift acknowledgements unchanged during preparation. The S3 source-content digest changes when RGB is ported: a passing test suite cannot supply a human acknowledgement or extend the old nine-register reset-state capture to RGB/touch. Publish that tranche only after the required acknowledgement is explicitly resolved. Do not import fork acknowledgements or alter gates to make the port pass.

**Next concrete work:** compose retained-net + autonomous-event patches on a base reconciled with #1727, carry the validation guards with the feature, and prepare the sensor fixtures. Keep RGB transport, the board model and the optional browser pad API separately reviewable, with the acceptance evidence and S3 gate decision attached.
