# WeAct BluePill-Plus-CH32 (CH32V203C8T6)

**Demand position:** Tier C vendor bet (WCH), downstream of CH32V003. The
CH32V203 has no row in the JetBrains State of Embedded survey; it is the WCH
QingKe V4B part that hobbyists use as a RISC-V "blue pill". This is the second
WCH part and the first RV32IMAC (V4-series) QingKe profile in LabWired.

## What this profile runs

Firmware boots RV32IMAC code from the CH32V203C8T6 memory map (64 KB flash at
0x08000000, 20 KB SRAM), stays on the reset 8 MHz HSI, enables GPIOA, GPIOB
and USART1 through RCC_APB2PCENR, configures PA9 as alternate-function
push-pull, sets 115200 baud, polls TXE, and sends `CH32V203_BLUEPILL_PLUS_OK`.
It then toggles the blue user LED on PB2. The smoke asserts the serial marker
and at least eight PB2 edges; the committed run recorded 34.

The UART output is pad-gated: rebuilding the same firmware without the PA9
alternate-function setting produces no UART bytes and the smoke fails.

The engine reuses its STM32F1-shaped RCC, GPIO and USART models for the
exercised subset, because the CH32V20x registers used here share their
offsets and bit positions. This is not a claim that the CH32V203 clock tree
or other peripherals are STM32F1-compatible.

**Not modeled:** PFIC/interrupts, PLL/HSE clock setup (WCH's `SystemInit`
raises the clock to 96/144 MHz), the PA0 user key, USB-C and CAN (both: see the
ch32fun profile below), timers and ADC (all three: see the Arduino core
section below),
RTC, the unplaced SPI flash, WCH SDI debug, and physical-board parity.

## Copy-paste example (WCH SDK / MounRiver Studio)

From WeAct's [CH32V203 template](https://github.com/WeActStudio/WeActStudio.BluePill-Plus-CH32/tree/master/Examples/CH32V203/01-template).
Connect PA9 to a USB-UART adapter's RX input at 115200 8N1.

```c
#include "debug.h"

int main(void) {
    GPIO_InitTypeDef gpio = {0};

    Delay_Init();
    USART_Printf_Init(115200);          /* USART1, TX on PA9 */
    printf("CH32V203_BLUEPILL_PLUS_OK\r\n");

    RCC_APB2PeriphClockCmd(RCC_APB2Periph_GPIOB, ENABLE);
    gpio.GPIO_Pin = GPIO_Pin_2;          /* blue LED, active high */
    gpio.GPIO_Mode = GPIO_Mode_Out_PP;
    gpio.GPIO_Speed = GPIO_Speed_50MHz;
    GPIO_Init(GPIOB, &gpio);

    for (;;) {
        GPIO_SetBits(GPIOB, GPIO_Pin_2);
        Delay_Ms(250);
        GPIO_ResetBits(GPIOB, GPIO_Pin_2);
        Delay_Ms(250);
    }
}
```

The simulator smoke uses the bare register-level equivalent in
[`firmware.rs`](firmware.rs), not the WCH SDK, because the SDK's startup
raises the clock through the PLL, which this profile does not model.

## Run the smoke

```bash
examples/weact-bluepill-plus-ch32v203/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/weact-bluepill-plus-ch32v203/onboarding-smoke.yaml
```

Evidence: [`evidence/`](evidence/) (run log, UART transcript, result JSON,
availability receipt).

## Unmodified ch32fun on the QingKe V4 profile

A second chip profile, [`ch32v203c8t6-ch32fun-smoke`](../../configs/chips/ch32v203c8t6-ch32fun-smoke.yaml),
runs [ch32fun](https://github.com/cnlohr/ch32fun) (MIT) CH32V20x examples
built with ch32fun's own compile and link lines and with no source changes
([`build-ch32fun-examples.sh`](build-ch32fun-examples.sh)). The one exception
is canbus_network's second node: it sets `#define TRANSMIT 1`, which is the
edit the example's README asks for. The ELFs are
committed in [`ch32fun/`](ch32fun/).

It adds what those examples need and the narrow profile above lacks:

- **The QingKe V4 core profile** (`core: qingke-v4b`, the name ch32-rs/ch32-data
  uses). It accepts the CORECFGR CSR (0xBC0) that ch32fun's startup writes,
  and it takes PFIC interrupts through the mtvec mode-3 address table, using
  the QingKe V2 trap path. A generic RV32 core traps on that CSR at reset.
- **The QingKe V4 SysTick** (`qingke_v4_systick`). It has a 64-bit CNT and CMP,
  counts up or down (MODE), and supports INIT. ch32fun's `DelaySysTick` polls
  the 64-bit counter.
- **The HSI ×18 PLL handshake and EXTEN_CTR.PLL_HSI_PRE** that ch32fun's
  SystemInit performs. The model states the resulting 144 MHz as `cpu_hz`; it
  does not derive it.
- **EXTI on the PFIC** (`profile: ch32v20x`). The sixteen GPIO lines drive
  PFIC 22..26, 39 and 56 as level requests, and AFIO_EXTICR1..4 select the port.
- **The WCH-LinkE debug printf channel** at 0xE0000380.
- **CAN1** (`bxcan` at 0x40006400, APB1PCENR bit 25). The CH32V20x CAN
  registers sit at the STM32F1 bxCAN offsets and bits, so the bxCAN model
  and the timed world CAN bus carry it: bit time comes from BTIMR and PCLK1,
  and every frame is acked, arbitrated and error-counted on the bus.
- **DMA1 on the PFIC** (`profile: ch32v20x`). Channel N's flags are PFIC
  level 26 + N, and a CNTR store on an enabled channel re-arms it, as on the
  CH32V003. SPI2 is the F1 register block on the F1 default pads (PB13 SCK,
  PB15 MOSI); `spi2_tx` is wired to channel 5.
- **USBFS (USBOTG_FS) in device mode** (`wch_usbfs` at 0x50000000, PFIC 59,
  AHBPCENR bit 12) with a scripted full-speed host on the cable. The
  controller is a bus master, as on silicon: SETUP and OUT packets are
  written to the endpoint buffers that `UEPn_DMA` points at in SRAM, and
  IN packets are read from them, under the `UEPn_TX/RX_CTRL` handshakes.
  The host enumerates the way an OS does. Its class requests and report
  polls come from the system manifest's `host_steps`, and everything it
  saw is the `usbfs` peripheral's `host` log. DATA0/DATA1 is checked: a
  wrong-PID IN packet is ACKed and discarded (USB 2.0 §8.6.4). ch32fun
  relies on this, because its control reads send a stale DATA0 packet
  first (see below). The RCC runs the F1 map under a `ch32v20x` profile
  that keeps AHBPCENR.OTG_FSEN; the F103 mask drops that bit.

| Smoke | ch32fun example | What it proves | Result |
| --- | --- | --- | --- |
| [`ch32fun-blink-smoke.yaml`](ch32fun-blink-smoke.yaml) | `examples_v20x/blink` | PA0/PA2/PA4/PA7 edges 36,000,015 cycles apart, i.e. 250.0001 ms at 144 MHz; RCC CFGR0 SWS=PLL, PLLMUL ×18 | 6/6 |
| [`ch32fun-exti-blink-smoke.yaml`](ch32fun-exti-blink-smoke.yaml) | `examples_v20x/exti_blink` | each rising PA1 edge enters EXTI1_IRQHandler (PFIC 23), which toggles PA10 ~30 cycles later; the release does nothing | 9/9 |
| [`ch32fun-debugprintf-smoke.yaml`](ch32fun-debugprintf-smoke.yaml) | `examples_v20x/debugprintfdemo` | `+0`, `-1[`, … `-5[` on the debug-printf terminal; the probe took every request | 5/5 |
| [`ch32fun-spidmatest-smoke.yaml`](ch32fun-spidmatest-smoke.yaml) | `examples_v20x/spidmatest` | SPI2 (16-bit, 1-line TX, 72 MHz) fed by DMA1 channel 5, circular: [`check-spidmatest.py`](check-spidmatest.py) decodes PB13/PB15 from the first SCK edge as 256 × `0xAAAA`, then 814 × `0x0AAF` (3.18 laps), the half-buffer refill DMA1_Channel5_IRQHandler (PFIC 31) writes on HT and TC | 8/8 |
| [`ch32fun-otg-device-smoke.yaml`](ch32fun-otg-device-smoke.yaml) | `examples_v20x/otg_device` | full-speed composite HID device on USBFS. The host enumerates it from the firmware's own tables: 1209:d035, address 5, 3 HID interfaces, the strings "CNLohr" / "ch32fun ch32v20x test" / "CUSTOMDEVICE000", SET_CONFIGURATION 1, and the three report descriptors. One round of the example's `testtop.hidapi`: a 255-byte feature report 0xAA goes out by SET_REPORT (4 OUT packets) and comes back byte-identical by GET_REPORT. Then the interrupt reports, including the one-shot mouse step `00 01 01 00` after SysTick bit 23 rises (466 ms) | 23/23 |
| [`can-network/test.yaml`](can-network/test.yaml) | `examples_v20x/canbus_network` (two nodes) | the README's two-node network on one timed CAN bus: node `rx` runs the example as committed (TRANSMIT 0), node `tx` the same source with TRANSMIT 1. `rx` prints `FIFO: 1/3: 8 bytes from 0x0BADC0DE: FE 7A BE E5 C0 FF EE 45`; `tx` TSTATR has RQCP0 + TXOK0; both ESRs are 0 | 7/7 |

Negative controls (committed with their logs):
- `evidence/ch32fun-blink-smoke/negative-control/`: with `core: rv32imac`,
  the core traps (mcause 2) on `csrw 0xbc0` at 0x1cc in `handle_reset`, and
  no pad toggles (0/6).
- `evidence/ch32fun-exti-blink-smoke/negative-control/`: with the EXTI on the
  STM32F1 profile, INTFR latches but no PFIC request is raised, and PA10
  never toggles (7/9).
- `evidence/ch32fun-spidmatest-smoke/negative-control/`: with DMA1 on the
  CH32V003 profile (channel lines 22..28), channel 5's HT/TC never reach
  PFIC 31, `sendbuff` keeps 0xAAAA (4/8), and the second lap on the wire is
  0xAAAA again.
- `evidence/ch32fun-canbus-network-wrong-bitrate/`
  ([`can-network/test-wrong-bitrate.yaml`](can-network/test-wrong-bitrate.yaml),
  6/6): the same two nodes on a 25 kbit/s bus. `rx` FIFO0 stays empty
  (REC 1, stuff error), `tx` ends with TERR0 (TEC 8, bit error). The example
  still prints "Message sent successfully", because its wait loop
  `while (!MessageSent(mb) && timeout--)` leaves `timeout` at 0xFFFFFFFF, so
  the `timeout == 0` branch never runs. That is a ch32fun bug, not a model
  artifact. Not reported upstream.

- `evidence/ch32fun-otg-device-no-trailing-in/`
  ([`ch32fun-otg-device-no-trailing-in.yaml`](ch32fun-otg-device-no-trailing-in.yaml),
  4/4): the same host script without the keyboard-LED SET_REPORT after
  GET_REPORT. After the 470 ms wait, endpoint 1 is never re-armed and the
  host times out, so the mouse step is lost.

**Two ch32fun USB behaviours the model exposes.** Both are in
`examples_v20x/otg_device/otgusb.c` as committed. Neither is a model
artifact.

1. *Every control read starts with a discarded packet.* The SETUP handler
   arms EP0 with `T_TOG` clear and does not copy the descriptor, so the
   first IN carries the SETUP bytes as DATA0. The host ACKs it and throws
   it away, because the first data packet must be DATA1. The IN interrupt
   then copies the descriptor and flips `T_TOG`, and the second IN carries
   it as DATA1. The host log counts these (`discarded 0 11`, one per
   GET_DESCRIPTOR). A host that did not check the PID would read the SETUP
   bytes as the device descriptor.
2. *After a control read, the interrupt endpoints stop until the next
   control transfer with an IN status stage.* The status OUT of every
   control read sets `USBOTG_errata_dont_send_endpoint_in_window`. Only an
   EP0 IN completion clears it, and while it is set `USBOTG_SendEndpoint`
   refuses to arm any endpoint. A Linux host's keyboard-LED SET_REPORT
   clears it, and so does testtop's next SET_REPORT. A host that stops
   after a GET_DESCRIPTOR or GET_REPORT gets no more reports (negative
   control above).

The firmware also fills endpoint 0x81 (declared as the keyboard's) with
the 4-byte mouse report, and 0x82 (declared as the mouse's) with the 8-byte
keyboard report. The host log records what was sent. These USB behaviours
are not reported upstream.

The USB host is a script, not an OS: it does what Linux usbhid and
testtop.hidapi do on the wire. Bus reset and attach debounce are shortened
(1 ms, 2 ms), and SOF and suspend are not modelled. `AUTO_TOG` is stored
but toggles nothing. WCH's EVT says it toggles endpoints 1..3, but ch32fun
sets it and also toggles by hand, which only works if it does nothing. Real
silicon has to settle which is right.

**CAN bit rate on the CH32V203.** canbus_network's `BAUD_25kbps` table assumes
PCLK1 = 144 MHz. That holds for its own Makefile target, the CH32V208
(`CH32V20x_D8`: ch32fun's SystemInit sets PPRE1 /1). On the CH32V203, SystemInit
sets PPRE1 /2, so PCLK1 = 72 MHz and the same BTIMR gives 12.5 kbit/s. The
example computes and prints this itself ("CAN Baud Rate: 12500bps"), so the
world bus runs at 12.5 kbit/s. A CH32V203 and a CH32V208 running this source
unchanged would run at different rates (12.5 and 25 kbit/s).

The world runner's `peripheral_log` assertion takes a `node:` for this, as
`memory_value` does, so a test can read each node's debug-printf terminal.

```bash
examples/weact-bluepill-plus-ch32v203/build-ch32fun-examples.sh   # optional: ELFs are committed
cargo run -q -p labwired-cli -- test \
  --script examples/weact-bluepill-plus-ch32v203/ch32fun-exti-blink-smoke.yaml
```

## WCH's official Arduino core (stock sketches)

The same `ch32v203c8t6-ch32fun-smoke` profile runs the Arduino matrix's stock
sketches built with WCH's official core,
[openwch/arduino_core_ch32](https://github.com/openwch/arduino_core_ch32),
through the community PlatformIO `ch32v` platform (`genericCH32V203C8T6`,
`-march=rv32imacxw`). The sketches are not modified for this board. The row
(`ch32v203` in [`boards.yaml`](../../validation/arduino-matrix/boards.yaml))
adds two build flags. `-DLED_BUILTIN=PB2` is needed because the CH32V203C8
variant leaves LED_BUILTIN undefined, and PB2 is this board's LED.
`-DLW_PWM_PIN=PA0` is needed because PB2 has no timer channel.

| Sketch | What it exercises | Result |
| --- | --- | --- |
| L0 / L1 | core boot, `Serial` on USART1 (PA9), `millis()` from the STK interrupt | ✅ ✅ |
| L2 | `digitalWrite(LED_BUILTIN)` on PB2, ≥2 edges | ✅ |
| L3 | `Wire` on I2C1 (PB6/PB7) to an INA219 at 0x40: config 0x399F, bus 0x19CA | ✅ |
| L4 | `SPI` on SPI1 (PA5/6/7, CS PA4) to a MAX31855: frame 0x01901600 | ✅ |
| L5 | `analogRead(A0)` on ADC1: two completed conversions (2, 3 on the F1 unseeded path) | ✅ |
| L6 | `analogWrite(PA0, 128)` on TIM2_CH1, ≥4 edges. The core's default resolution is 12-bit, so duty is 128/4096 = 3.125 %; the twin measured 4509 high cycles per 144009-cycle (1 kHz) period, 3.131 % | ✅ |
| L7 | `micros()` | ⏭️ upstream bug, below |
| L8 | bxCAN1 loopback (sketch's F1 register branch; CH32V20x keeps the map) | ✅ |

Evidence: [`evidence/arduino-matrix/`](evidence/arduino-matrix/) (scoreboard,
per-sketch UART log, result JSON and the test script). Reproduce:

```bash
cargo build -p labwired-cli --release
python3 validation/arduino-matrix/run_matrix.py --boards ch32v203
```

Getting the core to boot needed three things:

- **WCH XW instructions.** The core is built `-march=rv32imacxw`, and its
  prebuilt libraries use XW `c.lbu`/`c.sb`/`c.lhu`/`c.sh` and the sp-relative
  forms. On a QingKe profile, an RVC word that the base decoder leaves
  Unknown now goes through `decode_wch_xw`. That is the same decoder as the
  CH32V003 Arduino work (#1716). The first one the L0 image executes is
  `0xa44e` at 0x9b4.
- **The silicon-revision word at 0x40022030.** WCH's SPL reads it in
  `GPIO_Init`, `GPIO_PinRemapConfig` and `CAN_FilterInit`. If bits 27:24
  are 0, it takes the first-revision paths: GPIOC PC13..15 driven from
  CFGHR bits 2..0, and PCFR1 remap bits shifted. It is not in the RM's
  FLASH map; the EEVblog thread "CH32V203 Chip errata issue - PORTC" covers
  the same split. The model is current silicon, so it reads 0x0100_0000:
  the smallest value with a non-zero revision nibble. Nothing else is claimed
  ([`flash_chip_rev.yaml`](../../configs/peripherals/ch32v20x/flash_chip_rev.yaml)).
  Without it, L0 faults on the unmapped read at step 687.
- **I2C1, SPI1, ADC1 and TIM1..TIM4.** These are the F103 register
  layouts at the ch32v20x.h bases, with ch32v20x.h IRQ numbers (F1 + 16
  on the PFIC).

Negative controls ([`evidence/arduino-matrix/negative-controls/`](evidence/arduino-matrix/negative-controls/)):
L0 without the revision word: memory violation, no UART. L3 with no INA219
on the bus: `LW_L3_FAIL nack`. L6 watched on PA1 instead of PA0: oracle
fail. L8 with the CAN1 clock gate moved to bit 26: `LW_L8_FAIL`.

**Two upstream core behaviours, neither a model artifact.** Both are in WCH's
core as pinned (arduino_core_ch32 `3820604`). Neither has been reported upstream.

1. *`micros()` runs backwards inside each millisecond.* `getCurrentMicros()`
   (cores/arduino/ch32/clock.c) returns `ms*1000 + (CMP+1-CNT)*1000/(CMP+1)`,
   which assumes a down-counter. `hw_config.c` starts the STK with
   `CTLR = 0xF`, and MODE (bit 4) clear means count up. L7 measured
   t0=1999 t1=3999 t2=3929 t3=4999, and back-to-back reads gave 4985, 4851,
   4716 … while CNT rose. The CH32V003 row has the same skip.
2. *`%lu` prints nothing and shifts every later argument.* The toolchain's
   `libprintf.a` `print()` treats `l` as a 64-bit size. It 8-aligns the
   va_list, reads two words, and calls a 64-bit printer that is a stub
   (`li a0,0; ret`). `snprintf("%lu %u %d", 1234567UL, 42, -7)` gives
   `lu= u=42 d=1`. That is why L7's own FAIL line prints empty values.

**Not modelled in this profile:** I2C2, ADC2, the USBD block
at 0x40005C00, USBFS double buffering / isochronous / host mode / OTG, CAN2, the
CAN interrupts (the example polls), RTC,
the SPI2 HSCR high-speed bit (accepted, reads 0, no effect), USART1 output in the ch32fun
smokes (configured by ch32fun's UART printf setup, not asserted there; the
Arduino rows assert it),
PFIC nesting and V4 priority grouping, HSE, low-power modes, host-to-target
debug input, and physical-board parity. Exercised by nothing here, so
unclaimed: SPI1/I2C1 DMA, TIM remapped pads, TIM input capture and
encoder modes, ADC scan/injected groups. The PD0/PC0 toggles in
debugprintfdemo are not asserted: on LQFP48, PD0 is OSC_IN and PC0 is not
bonded.
