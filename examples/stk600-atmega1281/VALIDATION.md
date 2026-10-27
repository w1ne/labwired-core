# Validation — ATmega1281 on STK600

## Evidence

| Capability | Evidence | Result |
| :--- | :--- | :--- |
| Official kit combination | Microchip STK600 guide device support table | ATmega1281 + RC09 + SC02, TQFP64 |
| Playground board identity | configs/systems/stk600-atmega1281.yaml | Separate board profile reuses atmega1281 |
| Harness mapping and polarity | Manifest assertions in avr_stk600_atmega1281_firmware | led0 PB5 and sw0 PB0, both active-low |
| Firmware startup and USART0 | Same Rust integration test and io-smoke.yaml | STK600 1281 READY captured |
| GPIO behavior | Same Rust integration test | Released sw0 leaves LED0 high/off; pressed sw0 drives LED0 low/on |
| Reproducible target build | ci/build.sh | AVR-GCC -mmcu=atmega1281, F_CPU=16 MHz |
| AVR decode path check | Rust integration test | Pass: unsupported opcodes return DecodeError and fail the instruction stepping loop |
| Shared unsupported-instruction audit script | unsupported_instruction_audit.sh report | Unavailable for AVR: CLI rejected the architecture before executing instructions (exit 2) |
| Physical-board or silicon capture | None | Not claimed |

## Commands

    examples/stk600-atmega1281/ci/build.sh
    examples/stk600-atmega1281/ci/test.sh
    cargo test -p labwired-core --test avr_stk600_atmega1281_firmware -- --nocapture
    ./scripts/unsupported_instruction_audit.sh \
      --firmware tests/fixtures/avr/stk600-atmega1281.elf \
      --system configs/systems/stk600-atmega1281.yaml \
      --max-steps 200000 \
      --out-dir out/unsupported-audit/stk600-atmega1281

The shared audit wrapper was attempted with the command above. Its report records a ConfigError because labwired-cli run does not accept AVR targets; zero instruction steps were executed by that wrapper, so its zero-count opcode totals are not a pass. The dedicated Rust integration test does execute the firmware through Avr::step and would fail on an unsupported opcode. A general AVR audit lane remains unavailable.

The simulation clock is fixed at 16 MHz. USART0 is simulator-side serial capture, not an
emulation of the STK600's USB, RS-232, or board-level connector routing.
