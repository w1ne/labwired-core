# tier1-fixture-esp32s3 — Tier-1 matrix fixture

ESP32-S3 firmware that validates the simulator's chip model
peripheral-by-peripheral with raw register accesses (no esp-hal drivers —
esp-hal is used only for init/entry scaffolding) and reports one verdict
line per peripheral class over UART0.

## What it does

On boot the fixture runs one self-test per peripheral class — clock
(SYSTIMER), gpio, timer (TIMG0), irq (interrupt matrix), dma (GDMA m2m),
mcpwm, rmt, i2c — each against the documented ESP32-S3 TRM register
layout, and prints the TIER1 protocol:

```text
TIER1 <class> PASS
TIER1 <class> FAIL code=<reason>
TIER1 done
```

The `uart` class is implicit: receiving `TIER1 done` over UART0 is itself
the proof of a working UART path, so no `uart` line is ever printed.
Missing `done` degrades reported passes to `partial`; classes never
reported are `blocked`. The parser and row-resolution rules live in
`crates/cli/src/tier1.rs` (`parse_tier1_uart`, `ParsedTier1::resolve_row`).

Expected output on the current model:

```text
TIER1 clock PASS
TIER1 gpio PASS
TIER1 timer PASS
TIER1 irq PASS
TIER1 dma FAIL code=gdma-no-m2m-model
TIER1 mcpwm PASS
TIER1 rmt PASS
TIER1 i2c PASS
TIER1 done
```

The dma FAIL is honest: the GDMA model latches EOF without walking the
descriptor list (documented limitation in
`crates/core/src/peripherals/esp32s3/gdma.rs`), and the fixture verifies
the bytes actually moved — via volatile reads, so the check starts
passing the moment the model gains real memory-to-memory moves.

## Run in the simulator

The Tier-1 matrix harness runs the committed blobs from
`tests/fixtures/tier1/` on every chip target in
`labwired_cli::tier1::TIER1_TARGETS`:

```
cargo test -p labwired-cli --test tier1_matrix -- --nocapture
```

To run the fixture directly:

```
cargo run -p labwired-cli --release -- run \
    --chip configs/chips/esp32s3.yaml \
    --firmware tests/fixtures/tier1/esp32s3.elf \
    --max-steps 30000000 --rom-boot
```

with `LABWIRED_ESP32S3_FLASH=tests/fixtures/tier1/esp32s3-flash.bin` in
the environment (the `--rom-boot` path boots the real ROM + 2nd-stage
bootloader from the merged flash image).

## Rebuild the committed blobs

Needs the espressif Rust toolchain (`source ~/export-esp.sh`) and
`espflash`. One script rebuilds the ELF, regenerates the merged flash
image, and refreshes the sha256 manifest:

```
source ~/export-esp.sh
./scripts/build_tier1_fixtures.sh
```

This produces `tests/fixtures/tier1/esp32s3.elf`,
`tests/fixtures/tier1/esp32s3-flash.bin` (via
`espflash save-image --chip esp32s3 --merge` — the `--merge` flag is
required: a plain app image without bootloader + partition table does not
ROM-boot), and `tests/fixtures/tier1/MANIFEST.json` (sha256 + source rev
per blob, verified by the harness before every run).

## Sources

- ESP32-S3 TRM v1.4 §5 (GPIO), §13 (TIMG), §16 (SYSTIMER), §26 (UART),
  §29 (I²C)
- ESP-IDF `soc/esp32s3/register` headers (`gdma_reg.h`, `mcpwm_reg.h`,
  `rmt_reg.h`)
- Simulator model sources in `crates/core/src/peripherals/esp32s3/`
