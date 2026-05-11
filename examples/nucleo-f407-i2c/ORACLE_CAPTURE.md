# Capturing the F407 I²C Hardware Oracle

The `crates/core/tests/e2e_stm32f407_i2c.rs::aht20_bmp280_chip_id_handshake_matches_silicon`
test is gated `#[ignore]` until the JSON fixture at
`crates/core/tests/fixtures/stm32f407/aht20_bmp280_chip_id.json` is
populated with a register trace captured from real F407 silicon.

This document is the playbook for that capture. It is intentionally
not automated — capture happens once per firmware revision against
known hardware, and the JSON is a stable artifact thereafter.

## Hardware setup

- **MCU board:** Nucleo-F407 (or any STM32F407 dev board with SWD
  exposed) + ST-Link V2/V3 onboard programmer.
- **I²C devices:** AHT20 on `0x38`, BMP280 on `0x76`. SDO of BMP280
  must be tied low for `0x76`; tie high for `0x77` and update both
  `system.yaml` and the firmware.
- **Wiring:** I²C1 SCL on **PB6**, SDA on **PB7**. 4.7 kΩ pull-ups to
  3.3 V if the breakout boards don't carry them. Common ground.

## Software prerequisites

- `arm-none-eabi-gdb` (or `gdb-multiarch`).
- `openocd` with the `stm32f4x.cfg` target. On Pop!_OS / Ubuntu:
  ```bash
  sudo apt install openocd
  ```
- The firmware ELF built locally:
  ```bash
  cargo build --release -p nucleo-f407-i2c
  # produces target/thumbv7em-none-eabi/release/nucleo-f407-i2c
  ```

## Capture flow

1. **Flash the firmware** to F407 via ST-Link:

   ```bash
   openocd -f interface/stlink.cfg -f target/stm32f4x.cfg \
       -c "program target/thumbv7em-none-eabi/release/nucleo-f407-i2c verify reset exit"
   ```

2. **Start OpenOCD in server mode** in one terminal:

   ```bash
   openocd -f interface/stlink.cfg -f target/stm32f4x.cfg
   ```

   This exposes telnet on 4444 and GDB on 3333.

3. **Attach GDB** in another terminal:

   ```bash
   arm-none-eabi-gdb target/thumbv7em-none-eabi/release/nucleo-f407-i2c \
       -ex "target remote :3333" \
       -ex "monitor reset halt"
   ```

4. **Set breakpoints** at the points in `src/main.rs` where firmware
   issues each I²C register access — e.g. inside `i2c_start`,
   `i2c_send_address`, each `poll_sr1` exit, each `read_volatile(I2C1_DR)`
   and `write_volatile(I2C1_CR1, ...)` site.

5. **At each breakpoint**, read all five I²C registers + the bus
   peripheral's actual address access (the value of the operand to the
   triggering instruction). GDB can dump them in one go:

   ```gdb
   monitor mdw 0x40005400 9
   # 0x40005400 (CR1), 0x40005404 (CR2), 0x40005408 (OAR1), 0x4000540C (OAR2),
   # 0x40005410 (DR), 0x40005414 (SR1), 0x40005418 (SR2), 0x4000541C (CCR),
   # 0x40005420 (TRISE)
   ```

6. **For each captured operation**, append a `Write` or `Read` event to
   the trace JSON (`crates/core/tests/fixtures/stm32f407/aht20_bmp280_chip_id.json`).
   `i2c_base` is `0x40005400`; the `offset` field in each event is the
   register offset within that window (CR1=0x00, CR2=0x04, …, SR1=0x14,
   SR2=0x18). Use the schema in
   `crates/core/tests/e2e_stm32f407_i2c.rs::TraceEvent`.

7. **Inject `Tick` events** between operations that depend on
   state-machine progress (e.g. after the firmware sets the START bit,
   before it reads SR1 for SB). A small constant like 8–16 ticks per
   gap is usually enough; tune until the simulator replays clean.

## Verification

Once the fixture is populated:

```bash
cargo test -p labwired-core --test e2e_stm32f407_i2c -- --ignored
```

The test name `aht20_bmp280_chip_id_handshake_matches_silicon` should
go green. If it diverges, the error message names the step number,
register, expected vs. observed value — that's the exact point where
the simulator's I²C state machine disagrees with silicon.

## Minimal first-pass scope

For the first oracle capture, keep the scenario small and verifiable:

- **AHT20 init + read** — `0xBE 0x08 0x00` then status poll, plus the
  7-byte payload read.
- **BMP280 chip-ID read** — write `0xD0`, repeated start, read 1 byte;
  expected silicon response is `0x58`.

This is enough to certify "F407 I²C runs HAL-style transactions
byte-for-byte against silicon" and unlock F401 as a yaml-only port.
