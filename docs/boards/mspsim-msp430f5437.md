# MSP430F5437 onboarding scope

**Status: boots and passes its smoke on the MSP430X core.** SIM-DERIVED: no
MSP430F5437 bench part was measured. The evidence is the committed run under
`docs/coverage/renode-onboarding/metrics/mspsim-msp430f5437/`.

The descriptor `configs/chips/onboarding/mspsim-msp430f5437.yaml` was imported
from the mspsim chip-config `MSP430f5437Config.java`. It now declares only the
x5xx modules this engine implements. Register behaviour follows the
MSP430x5xx and MSP430x6xx Family User's Guide (SLAU208Q); addresses and vector
numbers follow the TI `msp430f5437.h` and `msp430f5437.ld` support files.

## CPU

`arch: msp430`, `core: msp430x`: the 20-bit CPUX of
`crates/core/src/cpu/msp430.rs`, the same core as the MSP430F2619 (see
`msp430f2619.md` for its scope and its comparison with the GDB simulator).

Limits: every instruction takes one clock cycle. No NMI is delivered: the
x5xx user NMI (`0xFFFA`) and system NMI (`0xFFFC`) vectors are never taken.
An unmapped access is a simulation error.

## Memory map

| Region | Address | Size | Model |
|---|---:|---:|---|
| Information memory | `0x1800` | 512 B | reads erased (`0xFF`) |
| RAM | `0x1C00` | 16 KiB | memory |
| Flash | `0x5C00` | 256 KiB | memory; vectors at `0xFF80`-`0xFFFF` |

## Modules

| Module | Address | Type | Modelled | Limits |
|---|---:|---|---|---|
| SFRIE1/SFRIFG1/SFRRPCR | `0x0100` | `msp430_sfr_x5` | flags and enables with reset values and reserved bits; WDTIE/WDTIFG request line 5 | OFIE, NMIIE, VMAIE, ACCVIE and the JTAG mailbox enables request nothing |
| PMM | `0x0120` | `msp430_pmm` | PMMCTL0 password (word key violation requests a PUC, byte lock and unlock), PMMCOREV, SVSMHCTL/SVSMLCTL delay flags, SVMLIFG and SVMLVLRIFG for a core voltage that settles at once | no voltage is simulated; high-side and reset flags are never set; PMMSWBOR and PMMSWPOR run as a PUC; no PMM interrupt |
| WDT_A WDTCTL | `0x015C` | `msp430_wdt_a` | password check; a key violation sets WDTIFG and requests a PUC | the counter does not run |
| UCS | `0x0160` | `msp430_ucs` | UCSCTL0 to UCSCTL8 with reset values and reserved bits; XT1 and XT2 fault flags and OFIFG for a board with no crystal | the FLL does not run; DCOFFG stays clear once cleared; no clock rate follows the registers |
| MPY32 | `0x04C0` | `msp430_mpy32` | MPY, MPYS, MAC, MACS with 16-bit and 32-bit operands, RES0 to RES3, SUMEXT, MPYC | MPYFRAC and MPYSAT do not change the result; the result is ready at once; RES2/RES3 after a 16-bit MAC are an assumption (the guide does not specify them) |
| USCI_A0 | `0x05C0` | `msp430_usci_a_x5` | UART mode, UCA0IE, UCA0IFG, UCA0IV, receive interrupt on line 6, `UCLISTEN` loopback | transmit is immediate; no receive errors; SPI, IrDA and auto-baud are storage |
| USCI_A1 | `0x0600` | `msp430_usci_a_x5` | same as USCI_A0, line 17 | same |

Not modelled and not mapped (an access is a bus error): the bootstrap loader
ROM at `0x1000`, digital I/O ports, Timer_A0/A1, Timer_B0, USCI_A2/A3,
USCI_B0 to B3, ADC12_A, DMA, RTC_A, CRC16, the flash controller, RAM control
and the SYS module (SYSRSTIV, SYSUNIV, SYSSNIV, the JTAG mailbox).

Firmware that blinks an LED, uses a timer tick or reads SYSRSTIV does not run
on this descriptor yet.

## Smoke

`examples/renode-onboarding/msp430f5437-uart-smoke.yaml` runs
`tests/fixtures/msp430f5437-uart-smoke.elf` (source
`examples/renode-onboarding/msp430-uart-smoke.c`, built with TI msp430-gcc
9.3.1.11 `-mlarge` by `scripts/build_msp430_uart_smoke.sh`). The firmware
prints one letter per check on USCI_A0:

1. MPY32 through compiler-generated code (16 x 16)
2. MPYS sign extension and MAC carry through the 16-bit registers
3. 32-bit division and remainder
4. OFIFG and XT1LFOFFG are set after power-on, come back while XT1 is the FLL
   reference and the ACLK source, and stay clear after both select REFO
5. polled UART loopback
6. receive interrupt in loopback (the handler reads UCA0IV)
7. LPM0 wake-up by the receive interrupt
8. a host byte through the receive interrupt
9. a call into flash above 64 KiB and a data read there
10. the PMM core voltage step to level 1 (`SetVCoreUp` of SLAU208 2.2.4), then
    a store to the locked PMMCTL0 that must not land
11. MPY32 with 32-bit operands: a 64-bit product through the compiler, a signed
    product and a MAC carry through the registers

Then it prints one line on USCI_A1 and writes WDTCTL without the password.
The restarted firmware finds WDTIFG set and reports it.

The smoke does not exercise the transmit interrupt, the watchdog interval
interrupt, the PMM software resets, the information memory, or XT2.
