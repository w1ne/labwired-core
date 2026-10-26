# STM32U5A5ZJ (NUCLEO-U5A5ZJ-Q)

The **STM32U5A5ZJT6Q** is the 4 MiB, 160 MHz Cortex-M33 variant carried by
ST's NUCLEO-U5A5ZJ-Q. It shares the MB1549 Nucleo-144 carrier with the
NUCLEO-U575ZI-Q. The U5A5 profile retains the U5 GPIO, USART1/2, ADC1, flash,
RTC, GPDMA1, ICACHE and FDCAN1 surfaces and adds behavioral HASH, SAES and PKA
models. Memory sizes and device bases follow ST's U5A5 CMSIS header and
DS13543; common peripheral descriptions reuse the U575/RM0456 register maps.

> **Fidelity: SIM-DERIVED.** No physical U5A5 board has been tested. The tests
> below exercise the simulator's register behavior and reference vectors; they
> do not establish silicon parity, cryptographic side-channel properties, or
> hardware performance.

## Board and memory

| Item | Model |
|------|-------|
| Chip descriptor | [`configs/chips/stm32u5a5zj.yaml`](../../configs/chips/stm32u5a5zj.yaml) |
| Board system | [`configs/systems/nucleo-u5a5zj.yaml`](../../configs/systems/nucleo-u5a5zj.yaml) |
| Example | [`examples/nucleo-u5a5zj/`](../../examples/nucleo-u5a5zj/) |
| Flash | 4 MiB at `0x08000000`, two 2 MiB banks |
| SRAM1 | 768 KiB at `0x20000000` |
| SRAM2 / SRAM3 | 64 KiB at `0x200C0000` / 832 KiB at `0x200D0000` |
| SRAM4 / SRAM5 | 16 KiB at `0x28000000` / 832 KiB at `0x201A0000` |
| Backup SRAM | 2 KiB at `0x40036400` |
| LEDs / B1 | PC7 / PB7 / PG2 active-high; PC13 active-high |
| ST-LINK VCP | USART1 PA9 TX / PA10 RX; simulator captures TX |

## Crypto peripherals

| Peripheral | Base | Executed operations | Current limits |
|------------|------|--------------------|----------------|
| HASH | `0x420C0400` | SHA-1, MD5, SHA-224, SHA-256 and HMAC from CPU-fed input; digest registers and completion status update on `STR.DCAL` | DMA pacing, sub-byte final blocks and hardware timing are not modeled |
| SAES | `0x420C0C00` | AES-128 and AES-256 encrypt/decrypt in ECB, CBC and CTR modes through key/IV, DINR and DOUTR | GCM/CCM, key derivation, TrustZone key selection, DMA and DPA behavior are not modeled |
| PKA | `0x420C2000` | Modular exponentiation, modular inverse/add/subtract/reduction, integer add/subtract/multiply and compare; operand widths up to 4160 bits | ECC, RSA-CRT, Montgomery helper modes and protected exponentiation report operation errors |

The operations are synchronous in this model. HASH and SAES set their modeled
completion state as soon as CPU input is complete; PKA sets completion on the
start write. Execution time and power are not represented. The PKA model is
integer arithmetic for firmware behavior, not a hardened cryptographic
implementation.

The test [`stm32u5_exec_oracle.rs`](../../crates/core/tests/stm32u5_exec_oracle.rs)
checks SHA-256 of `abc`, the AES-128 ECB NIST block vector, and
`4^13 mod 497 = 445` through the actual `SystemBus` register path. It also
checks negative integer subtraction sign extension through 4160 bits and rejects
operand widths above 4160 bits. Unit tests cover SHA-1/SHA-224/MD5, an RFC 4231 HMAC vector,
AES-128 ECB/CBC/CTR vectors, and PKA unsupported-mode status. The model implements
AES-256 ECB/CBC/CTR, but the current vectors do not cover AES-256.
[`stm32u5_reset_conformance.rs`](../../crates/core/tests/stm32u5_reset_conformance.rs)
checks the selected memory map and reset state.

## Playground

The Playground includes **NUCLEO-U5A5ZJ-Q** as a selectable Nucleo-144 target.
Its board drawing uses the shared MB1549 header layout, and its C/C++ compile
profiles use Zephyr's `nucleo_u5a5zj_q` board target. The pin map currently
reuses the U575 common GPIO/alternate-function subset; it is not a complete
U5A5 alternate-function inventory. The target is for simulation and firmware
experimentation; physical board acceptance is pending.

For the separate display demo, use the U575-based [Star Catcher
guide](../../../docs/guides/NUCLEO_U575ZI_Q_STAR_CATCHER.md). U575 has HASH but
does not have PKA or SAES; the U5A5 board is the crypto-capable Playground
choice.
