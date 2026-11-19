# MAX32652 EVKIT validation status

## Current measurement (2026-11-09)

The imported MAX32652 EVKIT system booted `firmware-max32652-smoke` for
**100,000 instructions** and passed **7/7 smoke assertions** (ten register
checks, `MAX32652 checks PPPPPPPPPP DONE`, summary 0xA32603FF at 0x200FF000,
one line on each of UART0-2, 6 edges on LED0 P2.25). Evidence is under
[`docs/coverage/renode-onboarding/metrics/max32652-evkit/`](../coverage/renode-onboarding/metrics/max32652-evkit/).
The chip surface is [`max32652.md`](max32652.md), included whole.

## Board I/O

Source: MSDK `Libraries/Boards/MAX32650/EvKit_V1` (`board.c` `led_pin` /
`pb_pin`, `board.h` `LED_ON 0`).

| id | Pad | Polarity |
|---|---|---|
| led0 | P2.25 | active low |
| led1 | P2.26 | active low |
| button0 | P2.28 | active low, weak pull-up |
| button1 | P2.30 | active low, weak pull-up |

Renode `max32652-evkit.repl` inverts led0 but not led1. The vendor BSP
drives both LEDs on with OUT_CLR, so both are declared active low.

The smoke runs its pull check on P1.7, not on a button pad: a declared
button holds its pad at the released level, so a pull-down on P2.28 reads
high on this board (first EVKIT run, check 6 failed for that reason).
