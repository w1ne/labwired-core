# Validation — CC2538DK (CC2538SF53)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no CC2538 bench part. `flash-controller.yaml` follows the steps of
Renode's `cc2538_flash_controller.robot` with an independent firmware (Renode's
test ELF is a prebuilt binary run against a TI ROM dump); it is not a run of
Renode's binary. The `crypto-*.yaml` scripts run the Contiki-NG v4.9
`cc2538-common/crypto` tests, built here; their program names and messages
are the ones Renode's `cc2538_cryptoprocessor.robot` waits for, and the
scripts assert a superset of its milestones. `cc2538_rpl_udp` runs the
Contiki-NG v4.9 `examples/rpl-udp` server and clients, built here, in the
three-node topology of Renode's `cc2538_rpl-udp.robot` and asserts the same
request/response lines (in Contiki-NG v4.9's wording). Renode's prebuilt
ELFs are not run. No wider Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the flash vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x200400, SP: 0x20008000` |
| Cortex-M3 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| SYS_CTRL reset clock (`CLOCK_STA`: HF-RC, `SYS_DIV`/`IO_DIV` 001) | smoke-tested | `clk osc=hfrc16 sys_div=1 io_div=1 sysclk=16000000`; unit tests pin the Table 7-3 reset values |
| SYS_CTRL `RCGCUART` gate write + read-back | smoke-tested | `uart0 rcgc=1 …` |
| SYS_CTRL gating effect on other blocks | **not modelled** | gates are storage only; UART0 works with the gate clear too |
| UART0 (PL011-compatible) TX @ 115200 from 16 MHz, `IBRD 8` / `FBRD 44` | smoke-tested | `uart0 … ibrd=8 fbrd=44` + heartbeat lines on the console sink |
| GPIO address-masked `GPIO_DATA` write + read | smoke-tested | `led=yellow on pin=1 leds=02` … (one masked store per LED; the nibble read proves no other pin moved); unit tests cover the SWRU319C Figure 9-2/9-3 examples |
| GPIO `GPIO_DIR` / `GPIO_AFSEL` | smoke-tested | LEDs driven only after DIR; PA0/PA1 handed to UART0 |
| Key inputs at the reset IOC pull-up | smoke-tested | `keys select=1 left=1 right=1 up=1 down=1` |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| Stock Contiki-NG v4.9 hello-world boots and runs its etimer | smoke-tested | `contiki-hello.yaml` 2/2 checks, 10,000,000 steps: banner, `Link-layer address: 0012.4b00.0000.0001`, 3 × `Hello, world`; `firmware_survival::test_cc2538dk_contiki_hello_survival` |
| Clean-room ROM: CCA boot (VTOR, SP, reset ISR) and parking on an invalid CCA | unit-tested on the CPU model | `cc2538_rom::rom_boot_*` |
| Clean-room ROM utility API at `0x48` (crc32, memset/memcpy/memcmp/memmove, chip id and flash size from `DIECFG0`) | unit-tested on the CPU model | `cc2538_rom` (CRC-32 check value `0xCBF43926`) |
| ROM `PageErase`/`ProgramFlash` through FLASH_CTRL: multi-page erase, program + NOR AND rule, `-2` on bad parameters, `-1` on a CCA-locked page or the lock-bit page without `UPPER_PAGE_ACCESS`, 20-ms erase | integration-tested on the CPU model | `cc2538_rom` (12 tests, with and without `event-scheduler`) |
| FLASH_CTRL registers: `FCTL` busy/full/abort, read-only status, `FADDR` word read-back and roll-over, `FWDATA` acceptance, `DIECFG0-2` of a CC2538SF53 | unit-tested | `cargo test -p labwired-core --lib cc2538::flash_ctrl` (8 tests) |
| Flash controller, the steps of Renode's `cc2538_flash_controller.robot` | smoke-tested | `flash-controller.yaml` 2/2 checks, 20,000,000 steps: `FLASH SIZE: 0x80000`, memset write, pattern write, one-page erase that keeps the neighbour, two-page erase; `firmware_survival::test_cc2538dk_contiki_flash_test_survival` |
| Stock Contiki-NG v4.9 CFS/Coffee (`examples/storage/cfs-coffee`) | smoke-tested | `contiki-test-cfs.yaml` 2/2 checks, 60,000,000 steps: `CFS test 1/2 completed with 0 errors`; `contiki-coffee.yaml` 2/2 checks: write, read back, append, directory listing `test (34 bytes)` |
| Negative control: the old ROM (`PageErase`/`ProgramFlash` returned -1) | fails | `LABWIRED_CC2538_ROM=<old cc2538_rom.bin>`: test-cfs `failed to write at offset 1920` and 3 errors, example-coffee reads `""`, flash test prints `[FAIL]` ×4 |
| Instruction support for the CFS image | audited | `unsupported_instruction_audit.sh`: 20,000,000 steps, 0 unsupported |
| Determinism of the flash runs | smoke-tested | two runs of each flash script, identical `uart.log` and `snapshot.json` |
| AES/SHA-256 registers: Table 22-103 reset values, key store (contiguous free areas, even start for 192/256, write-1-to-free, size write deletes keys, read errors), SAVED_CONTEXT_READY cleared by the 4th TAG word but not by a peek, level IRQ `INT_STAT & INT_EN` | unit-tested | `cargo test -p labwired-core --lib cc2538::aes` (13 tests) |
| AES modes against published vectors: ECB (FIPS-197 C.1-C.3, all key sizes), CBC and CTR (SP 800-38A F.2.1/F.5.1), CCM (RFC 3610 #1, both directions), GCM (GCM spec test case 4), CBC-MAC, SHA-256 (FIPS 180-4 "abc", resumed two-block message) | unit-tested | same 13 tests |
| Stock Contiki-NG v4.9 `cc2538-common/crypto` suite: CBC 16, CBC-MAC 16, CTR 6, CCM 22 (incl. AAD-only and `mic_len` 0), ECB 16, GCM 8+8, SHA-256 9 vectors | smoke-tested | `crypto-{cbc,cbc-mac,ctr,ccm,ecb,gcm,sha256}.yaml`, 2/2 checks each, 30,000,000 steps; every vector's `...(): success` and `... OK` lines in order; `firmware_survival::test_cc2538dk_contiki_crypto_{ccm,sha256}_survival` (with and without `event-scheduler`) |
| Renode `cc2538_cryptoprocessor.robot` milestones | superset | all 48/64/18/99 `Wait For Line On Uart` lines of its CBC/CBC-MAC/CTR/CCM cases appear, in order, in `crypto-{cbc,cbc-mac,ctr,ccm}.yaml` |
| Negative controls | fail | the old `stub` at `0x4008_B000`: every crypto image hangs at `Filling 128-bit key store...`. Model mutations: no level re-pend → CCM hangs at the AAD-only vector #1; CCM B0 without the Adata flag → `MIC does not match` / `authentication failed`; SHA-256 resume without the digest byte swap and GCM without the ciphertext length → the scripts fail |
| Instruction support for the crypto images | audited | `unsupported_instruction_audit.sh` on `crypto-ccm`: 30,000,000 steps, 0 unsupported |
| Determinism of the crypto runs | smoke-tested | two runs of each of the seven scripts, identical `uart.log` and `snapshot.json` |
| Cryptoprocessor slave-interface data path (`AES_DATA_IN_OUT`, `HASH_DATA_IN`, `INPUT_READY`/`OUTPUT_READY`/`RFD_IN`), `KEY2`/`KEY3`, pulse interrupts, `PROT_EN`, DMA/processing latency | **not modelled** | `crates/core/src/peripherals/cc2538/aes.rs` |
| SYS_CTRL `CLOCK_STA.SYNC_32K` | exercised | Contiki-NG `sys_ctrl_init` waits for its edge |
| RF core: strobes, main-FSM timing, RSSI/CCA, RFRND, MTCTRL, TX timing | unit-tested; exercised | `cargo test -p labwired-core --lib cc2538::rfcore` (9 tests); Contiki `random_init` + radio init (strobes, RSSI_VALID, RFRND, CCA, MTCTRL) |
| Sleep timer: 32-kHz count and ST0 latch | unit-tested; exercised | `cargo test -p labwired-core --lib cc2538::sleep_timer` (8 tests); Contiki reads the count (5,721 `ST0` latches in the 10M-step run) |
| Sleep timer compare → SM Timer IRQ 145 → handler | integration-tested | `cc2538_sleep_timer_irq` on the descriptor-built bus, with and without `event-scheduler`. The stock hello-world never arms a compare: CSMA keeps the radio listening, so LPM stays in PM0 |
| Instruction support for the Contiki image | audited | `unsupported_instruction_audit.sh`: 10,000,000 steps, 0 unsupported |
| Determinism of the Contiki run | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Three nodes on a shared 802.15.4 air: stock Contiki-NG v4.9 `examples/rpl-udp` (one `udp-server` RPL root, two `udp-client`s on one ELF, per-node IEEE address in the info page as the Robot test writes it) | integration-tested | `cc2538_rpl_udp` (`--features event-scheduler`; PR lane "Scheduler-only fidelity lanes"): both clients join the DODAG and log `Sending request i` / `Received response 'hello i'` for i = 0..2, the server logs `Received request 'hello i' from fd00::200:0:0:{2,3}` + `Sending response.` for each, and hardware ACK frames cross the air. 27.6 s simulated, ~8 s release / ~11 s debug |
| Renode `cc2538_rpl-udp.robot` milestones | matched | its 3 × 4 server lines and 3 × 2 lines per client, in Contiki-NG v4.9's wording (`Received request 'hello 0' from …`, `Sending response.`, `Received response 'hello 0' from …`) |
| 802.15.4 air: FCS (CRC-16/KERMIT check value), SWRU319C §23.9.5.1 frame filtering, AUTOACK decision, broadcast delivery after the last bit, listen-from-first-bit | unit-tested | `cargo test -p labwired-core --lib ieee802154_air` (5 tests) |
| RF core on the air: TX of the FIFO MPDU + FCS, RX FIFO layout (Figure 23-13: length, MPDU, RSSI, `CRC_OK`/correlation), FIFO/FIFOP/SFD/CCA, `RFIRQF0`/`RFIRQF1.TXDONE`, IRQ 141 level, own frames and other channels ignored, `ISFLUSHRX` | unit-tested | `cargo test -p labwired-core --lib cc2538::rfcore` (12 tests) |
| µDMA: software-requested basic/auto transfers (fixed and incrementing pointers), `XFERMODE`/`XFERSIZE` write-back, `ENASET` clear, `CHIS` + IRQ 46, `MASTEN`/enable gating, bus error → `ERRCLR` | unit-tested; exercised | `cargo test -p labwired-core --lib cc2538::udma` (4 tests); Contiki-NG's cc2538-rf TX (into `RFDATA`) and RX (out of `RFDATA`) |
| Negative controls (RPL-UDP) | fail | `udma` back to `type: stub` → no frame is ever sent; the air never acknowledges → the server never sees a request |
| Determinism of the RPL-UDP run | integration-tested | two runs, identical consoles on all three nodes |
| RF path loss, range, collisions, capture; the RF error interrupt; CSP; MAC-timer count; source matching / AUTOPEND; `FIFOPCTRL` thresholds | **not modelled** | lossless air: every node hears every other (Renode's range-11 topology puts the clients out of each other's range); see `crates/core/src/peripherals/ieee802154_air.rs` and `cc2538/rfcore.rs` |
| µDMA peripheral requests, channel routing (`CHASGN`/`CHMAP`), ping-pong / scatter-gather, arbitration, transfer time, error IRQ | **not modelled** | `crates/core/src/peripherals/cc2538/udma.rs` |
| ROM serial boot loader | **not modelled** | ROM parks in WFI |
| Flash: CPU stall while executing from flash during an operation, flash DMA trigger, flash interrupt, the 8-writes-per-word limit, mass erase | **not modelled** | `crates/core/src/peripherals/cc2538/flash_ctrl.rs` |
| Watchdog, sleep-timer capture | **not modelled** | `WDCTL`/`STCC`/`STCS` are storage |
| IOC pin mux / pad overrides | **not modelled** | the IOC window is a stub |
| GPIO interrupts, AFSEL commit lock | **not modelled** | `GPIO_RIS`/`GPIO_MIS` read 0 |
| CCA in the smoke image | pinned | the loader enters the smoke at its flash-base table; `test_cc2538dk_smoke_customer_configuration_area_boots` pins its 44 bytes |
| PKA, SSI, I2C, GPTimers, USB, ADC | declared stubs | `configs/chips/cc2538sf53.yaml` |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-cc2538dk-smoke --release --target thumbv7m-none-eabi
#   Finished `release` profile — no warnings

./examples/cc2538dk/ci/test.sh
#   PASS  3/3 checks · io-smoke · 4000000 steps
#   Audit summary: unknown_thumb16: 0 · unhandled_thumb32: 0 · unsupported_total: 0

./target/release/labwired \
  --firmware target/thumbv7m-none-eabi/release/firmware-cc2538dk-smoke \
  --system configs/systems/cc2538dk.yaml --max-steps 400000
#   Initial PC: 0x200400, SP: 0x20008000
#   CC2538DK SMOKE
#   cpu=cortex-m3 uart=uart0
#   clk osc=hfrc16 sys_div=1 io_div=1 sysclk=16000000
#   uart0 rcgc=1 ibrd=8 fbrd=44
#   led=yellow on pin=1 leds=02
#   ...
#   hb=0 yellow=1 green=0 orange=0

# Determinism: two runs, identical UART + snapshot
cmp out/cc2538dk-run1/uart.log out/cc2538dk-run2/uart.log          # clean
cmp out/cc2538dk-run1/snapshot.json out/cc2538dk-run2/snapshot.json  # clean

cargo test -p labwired-core --lib cc2538
#   47 passed (GPIO + SYS_CTRL as before; rfcore 9; sleep_timer 8; flash_ctrl 8; aes 13)
cargo test -p labwired-core --test cc2538_sleep_timer_irq
cargo test -p labwired-core --features event-scheduler --test cc2538_sleep_timer_irq
#   1 passed each (legacy walk / scheduled event)
cargo test -p labwired-core --test cc2538_rom
cargo test -p labwired-core --features event-scheduler --test cc2538_rom
#   12 passed each
cargo test -p labwired-core --test firmware_survival cc2538
#   test_cc2538dk_smoke_survival, test_cc2538dk_contiki_hello_survival,
#   test_cc2538dk_contiki_flash_test_survival,
#   test_cc2538dk_contiki_crypto_ccm_survival,
#   test_cc2538dk_contiki_crypto_sha256_survival ok

# AES/SHA-256 cryptoprocessor (2026-10-31)
for t in ecb cbc ctr cbc-mac gcm ccm sha256; do
  ./target/release/labwired test --script examples/cc2538dk/crypto-$t.yaml \
    --output-dir out/cc2538dk-crypto-$t --no-uart-stdout
done
#   PASS  2/2 checks · crypto-ecb · 30000000 steps
#   PASS  2/2 checks · crypto-cbc · 30000000 steps
#   PASS  2/2 checks · crypto-ctr · 30000000 steps
#   PASS  2/2 checks · crypto-cbc-mac · 30000000 steps
#   PASS  2/2 checks · crypto-gcm · 30000000 steps
#   PASS  2/2 checks · crypto-ccm · 30000000 steps
#   PASS  2/2 checks · crypto-sha256 · 30000000 steps
#   e.g. crypto-ccm:
#   Test vector #1: encrypt
#   len_len=2 key_area=1
#   adata_len=26 mdata_len=0 mic_len=8
#   ccm_auth_encrypt_start(): success, 30 us
#   ccm_auth_encrypt_check_status() wait: 0 us
#   ccm_auth_encrypt_get_result(): success, 0 us
#   MIC OK

# Flash controller (2026-10-30)
./target/release/labwired test --script examples/cc2538dk/flash-controller.yaml \
  --output-dir out/cc2538dk-flash --no-uart-stdout
#   PASS  2/2 checks · flash-controller · 20000000 steps
#   CC2538 FLASH TEST
#   FLASH SIZE: 0x80000
#   [OK] Data written with memset successfully
#   [OK] Data written successfully
#   [OK] Data erased successfully
#   [OK] Data erased successfully
#   FLASH TEST DONE
./target/release/labwired test --script examples/cc2538dk/contiki-test-cfs.yaml \
  --output-dir out/cc2538dk-cfs --no-uart-stdout
#   PASS  2/2 checks · contiki-test-cfs · 60000000 steps
#   CFS test 1 completed with 0 errors
#   CFS test 2 completed with 0 errors
./target/release/labwired test --script examples/cc2538dk/contiki-coffee.yaml \
  --output-dir out/cc2538dk-coffee --no-uart-stdout
#   PASS  2/2 checks · contiki-coffee · 20000000 steps
#   Wrote message "The first test", sequence 1
#   Read message "The first test", sequence 1
#   Wrote message "The second test", sequence 2
#   Read message "The first test", sequence 1
#   Read message "The second test", sequence 2
#   Available files
#   test (34 bytes)

./target/release/labwired test --script examples/cc2538dk/contiki-hello.yaml \
  --output-dir out/cc2538dk-contiki --no-uart-stdout
#   PASS  2/2 checks · contiki-hello · 10000000 steps
#   [INFO: Main      ] Starting Contiki-NG-release/v4.9
#   [INFO: Main      ] - Routing: RPL Lite
#   [INFO: Main      ] - Net: sicslowpan
#   [INFO: Main      ] - MAC: CSMA
#   [INFO: Main      ] - 802.15.4 PANID: 0xabcd
#   [INFO: Main      ] - 802.15.4 Default channel: 26
#   [INFO: Main      ] Node ID: 1
#   [INFO: Main      ] Link-layer address: 0012.4b00.0000.0001
#   [INFO: Main      ] Tentative link-local IPv6 address: fe80::212:4b00:0:1
#   [INFO: CC2538DK  ] TI SmartRF06 + cc2538EM
#   Hello, world
#   ...
# A 20M-step run prints 27 lines over 268.2 simulated s: one at boot, one per
# 10-s etimer period.
```

## Notes

- **Toolchain.** The CC2538 is a Cortex-M3 (ARMv7-M), and the firmware is built
  for `thumbv7m-none-eabi`, so the compiler cannot emit ARMv7E-M DSP or FPU
  instructions. The unsupported-instruction audit runs on that ELF.
- **Clock.** The firmware never leaves the reset clock. CLOCK_CTRL is not
  written; the divisors come from what CLOCK_STA reports, and the baud
  arithmetic is in the firmware header (achieved 115,108 baud, −0.08 %).
- **LED1 / PC0** is declared in `board_io` but never driven: it doubles as the
  CC2538 USB D+ pull-up on this board.
- **Bit 3 of `CLOCK_STA`.** Table 7-3 gives the reset value `0x0101_0109`
  while the bit-field table calls bits 7:3 reserved (0). The model returns the
  summary value verbatim.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The staged `configs/chips/onboarding/cc2538.yaml` (flash at `0x08000000`,
  128 KiB; 256 KiB RAM) did not match SWRU319C Table 4-1. It is kept, marked
  superseded, because `crates/core/src/bus/known_stubs.rs` still cites it.
