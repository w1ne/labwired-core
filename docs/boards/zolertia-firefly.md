# Zolertia Firefly validation status

## Current measurement (2026-11-09)

The imported Zolertia Firefly system booted `firmware-zolertia-firefly-smoke`
for **400,000 instructions** and passed **7/7 smoke assertions**: nine
register checks (`FIREFLY checks PPPPPPPPP DONE`), summary 0xF1EF01FF at
0x20007F00, 8+ edges on the red LED (PD5), one line on UART1, and
`fidelity_clean`. Evidence is under
[`docs/coverage/renode-onboarding/metrics/zolertia-firefly/`](../coverage/renode-onboarding/metrics/zolertia-firefly/).

## Chip surface

The board descriptor includes the modelled `configs/chips/cc2538sf53.yaml`
whole (UART0/1, GPIO A-D, SYS_CTRL, IOC, flash controller, sleep timer,
SOC_ADC, SSI0/1, I2C, GPTimer0-3, RF core, AES, uDMA, clean-room ROM).
Renode `platforms/boards/zolertia-firefly.repl` is `cpus/cc2538.repl` plus a
CC1200 on `ssi0`.

Not declared: the CC1200 sub-GHz radio. LabWired has no CC1200 model.

## Smoke checks

| # | Check | What it proves |
|---|---|---|
| 1 | clk | CLOCK_STA reports the 16-MHz HF-RC reset clock |
| 2 | leds | each LED on/off through a one-pin GPIO_DATA mask; the 3-pin mask reads only the written pin |
| 3 | dir | GPIO_D DIR reads back the LED mask |
| 4 | button | PA3 reads 1 (released) |
| 5 | ssi0 | SSI0 internal loopback returns 0xA5, 0x3C |
| 6 | gpt0 | GPTimer0 32-bit periodic: three time-outs |
| 7 | sleep | the 32-kHz sleep timer advances |
| 8 | uart1 | UART1 divisors read back (IBRD 8, FBRD 44) |
| 9 | sram | a pattern at the top of the 32 KiB SRAM reads back |

Check 4 does not prove the button model: the IOC reset pull-up also reads 1.

## Board I/O

Source: Contiki-NG `arch/platform/zoul/firefly/board.h` and
`arch/platform/zoul/dev/leds-arch.c` (`negative_logic = false`).

| id | Pad | Polarity |
|---|---|---|
| led_red | PD5 | active high |
| led_green | PD4 | active high |
| led_blue | PD3 | active high |
| button_user | PA3 | active low (shared with the boot-loader backdoor) |

Renode's `zolertia-firefly.repl` declares no LEDs or button.

## Negative controls

Under `docs/coverage/renode-onboarding/attempt-history/zolertia-firefly/`:

- `2026-11-09-negative-control-old-descriptor`: the previous descriptor
  (CPU, memory, UART0 only). BusFault at 0x400D2028 (SYS_CTRL), 1/7.
- `2026-11-09-negative-control-no-gpiod`: this descriptor without GPIO D.
  BusFault at 0x400DC420 (GPIO_D AFSEL), 1/7.
