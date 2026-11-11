# Prepared ESP32-S3 RGB display and touch mergeback

This is an **inactive review artifact**. The patch adds functional RGB565,
ST7701 and GT911 models. Storing the patch does not apply its engine changes.
The active-source candidate remains blocked by the normal ESP32-S3
source-content drift gate; no acknowledgement or hardware evidence was changed.

## Exact candidate

| Item | Value |
| --- | --- |
| Tested prerequisite base | `b82bc13b3737447fd32e6ade1725d8dd34624623` |
| Complete candidate Git tree | `8eeba279b0bc36aef421a5ac93f24a11ea64df77` |
| Patch | [esp32s3-rgb-touch.patch](esp32s3-rgb-touch.patch) |
| Patch SHA-256 | `8edd9c3314cf1de517e2546f8b8594034d8af6893aabcee6ea29fe353154394f` |
| Patch size | 92,395 bytes |
| Detailed receipt | [esp32s3-rgb-touch.validation.json](esp32s3-rgb-touch.validation.json) |

The binary patch uses zero context to avoid whitespace-only context lines in the
review artifact. Applying it to a separate index at the pinned base reproduced
the candidate tree exactly. The receipt records every changed file's SHA-256,
commands, results, expected-failure controls, firmware, and exact JS/WASM hashes.

```sh
git apply --unidiff-zero --check docs/engineering/mergeback/esp32s3-rgb-touch.patch
```

## Scope and source

The selective MIT-licensed source is
[CrispStrobe 1843597](https://github.com/CrispStrobe/labwired-core/commit/184359726288d048ef315d4fa4e99f1f737208a0),
reviewed at snapshot `d543a46bba894f9e7e3aa31cdba6feb561b2d08e`.
The generic `attach_rgb_panel` capability comes from
[b627fafc](https://github.com/CrispStrobe/labwired-core/commit/b627fafc4e7bd9d853708b1a62495d7a32fe963b);
only the ST7701 and GT911 registrations come from
[0efad509](https://github.com/CrispStrobe/labwired-core/commit/0efad50949f8ab6c8e008d07b913ad363fd5658c).
Preserve this co-author credit when activating the port:
`CrispStrobe <154636388+CrispStrobe@users.noreply.github.com>`.

| Area | Resulting behavior |
| --- | --- |
| LCD_CAM and GDMA | Functional active-area RGB565 frames, odd descriptor boundaries, byte/bit ordering, finite or cyclic scanout, ownership/writeback and descriptor errors. |
| ST7701 | GPIO 9-bit control and RGB frame receiver; reset, sleep, display and backlight control visible artifacts. |
| GT911 | Single-contact I2C register protocol, coherent ready/report/ack handling, and simulation-time held-contact refresh. |
| Integration | Existing generic peripheral kit, display artifact and input interfaces; two generated kit catalog entries. |
| Fixture | Source-built Xtensa ELF initializes the devices, reads touch through actual I2C commands, and redraws through DMA. |

The system profile uses existing `configs/chips/esp32s3.yaml`. It adds no chip
catalog alias, performance baseline, waiver, silicon tier or device-specific MCP
API. The fork's optional external GPIO-drive API and unrelated pad tests are
outside this patch.

Independent review found a reset defect in the source port: switching to i80
before `LCD_RESET` could leave a pending RGB frame and retire a later i80
transaction. The added regression fails against the initial port. The correction
makes reset clear transfer state regardless of selected mode and makes a fresh
i80 start discard stale RGB completion. Review accepted that narrow correction.

## Measured software qualification

All final runs below use the pinned combined prerequisite source and corrected
LCD model. The receipt contains the exact command arguments and log hashes.

| Check | Observed result |
| --- | --- |
| RGB suite, default | 9 passed, including the mode-transition regression. |
| Existing i80 suite, default | 6 passed. |
| RGB suite, event scheduler | 9 passed. |
| Existing i80 suite, event scheduler | 6 passed. |
| GT911 protocol unit tests | 3 passed. |
| Peripheral kit registry/catalog gate | 13 passed. |
| Generated catalog, model ratings, firmware exercise matrix | All three checks passed. |
| Existing S3 reset-state oracle | 1 passed, comparing the same nine recorded reset registers. |
| Initial port with new mode-transition regression | Expected failure observed before the correction. |
| RGB completion disabled | Guest failed to produce report 1, as required. |
| Touch forced released | Guest failed to produce report 2, as required. |
| Original source restored after both mutations | Byte hashes matched; the guest passed again. |
| Fresh Node/WASM engine | Initial display, down/move/up redraw, UART output and fresh-constructor reset passed. |
| Normal pre-commit hook | Formatting, all-target clippy and derived host check passed; only `esp32s3` drift acknowledgement blocked completion. |

The fixture rebuild reproduced **18,032 bytes** exactly, SHA-256
`fc5563c16da88dc609c5ada36b17c72a611cc4998675d502572318c1657c419f`.
The final test engine was built with `wasm-pack --dev --no-opt`; it is not a
release bundle or a browser UI qualification.

| Final engine artifact | SHA-256 |
| --- | --- |
| JavaScript | `2c84352644aa7ed51809c2ebdc7921a22a59c56dc58e0187cae93e245029b090` |
| WebAssembly | `36538be314235ad77b8e66d1c549a67245520ebd2b9f08ac6116dc21c26c80f8` |

The same guest runs through native and WASM APIs. Its asymmetric initial corners
arrive through guest-owned DMA descriptors, and pixel zero changes to the X
coordinate that firmware reads from GT911. Release restores red. Host code does
not paint the framebuffer.

## Remaining human decision

The rule is in [`validation/manifest.yaml`](../../../validation/manifest.yaml),
under **DRIFT** and **AN ACK LAPSES**, and in the `esp32s3` board record.
[`scripts/generate_validation_status.py`](../../../scripts/generate_validation_status.py)
enforces it in `evaluate`; `write_ack_digests` explicitly states that writing a
digest is not itself a human acknowledgement. The normal
[`scripts/pre-commit.sh`](../../../scripts/pre-commit.sh) runs that gate.

The current acknowledgement identifies earlier source content. Activation needs
a human to accept the exact changed digest and tracked recapture gap, or a real
capture that refreshes the evidence. An acknowledgement normally expires after
30 days. **No new acknowledgement, expiry extension, recapture owner or deadline
has been assigned by this work.**

| S3 validation value | Exact value |
| --- | --- |
| Existing acknowledgement date | `2026-11-10` |
| Existing acknowledgement digest | `c30213b480a08adac0801cc2ebe2947169beaadcd9bd2d11b8cfd601c1286102` |
| Proposed source-content digest | `a57ba52b2df8e004eb49b78bf7973e0e58782b9601b2b9417bdc452857d1f283` |
| Last recorded physical capture | `2026-08-09` |

That physical anchor is only **nine reset-state registers**. The recorded sweep
read 384 registers across ten windows, but only nine curated reset values are
asserted by the offline oracle. Passing that replay does not qualify RGB, touch,
physical timing or arbitrary Arduino/LVGL firmware.

## Proposed recapture follow-up

This is a proposed route, not a commitment on the user's behalf. The inspected
HIL registry marks `esp32s3-devkit` as `planned`; work-1 had no connected S3 USB
or serial device and no OpenOCD/esptool. No live hardware command was run.

1. Connect and identify an available ESP32-S3 USB-JTAG board and inspect its
   current use before reset/halt. Select the observed serial and Espressif
   OpenOCD installation.
2. Run the existing `scripts/hw-oracle/esp32s3_capture_estate.sh` with `OOCD_HOME`
   and `ESP32S3_SERIAL`, writing a new capture directory. Retain timestamp,
   board identity, tool version and logs; compare the same nine curated reset
   registers and record any discrepancies.
3. Update `silicon.last_capture`, result and source digest only after that real
   capture. A separate S040 behavioral workload and capture is needed for the
   new LCD_CAM/GDMA frame/ownership behavior and GT911 status/coordinate/ack path.
   A reset sweep must not be relabeled as RGB/touch validation.

## Functional limits

VSYNC is completed active-area DMA data. Pixel clock, porches, RGB pin-mux
waveforms and analog panel behavior are not modeled. GT911 is one contact with
a fixed 10 ms held-contact refresh; reset/address strapping, IRQ delivery,
gestures, multiple contacts and EEPROM persistence are absent. I2C pin-route
metadata does not prove wrong-pin NACK behavior. No physical-board, release,
throughput or arbitrary vendor-firmware claim follows from these tests.
