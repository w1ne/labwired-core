# ATmega1281 on STK600

**ATmega1281 on the Microchip STK600** — a board-level Playground system profile over
LabWired's existing ATmega1281 chip model.

!!! tip "Live status"
    The profile has a deterministic firmware smoke and CI registration. See the
    [target support rubric](../target_support_rubric.md) and the committed
    [validation record](../../examples/stk600-atmega1281/VALIDATION.md).

## Status at a glance

| Aspect | Status |
| --- | --- |
| Chip model | [ATmega1281 descriptor](../../configs/chips/atmega1281.yaml), shared with the bare-chip target |
| Board system | [stk600-atmega1281](../../configs/systems/stk600-atmega1281.yaml) |
| Official kit combination | STK600 RC09 routing card + SC02 TQFP64 socket card |
| Playground example | [stk600-atmega1281](../../examples/stk600-atmega1281/) |
| Support level | L1 smoke-supported, simulation-only |
| Clock | Fixed 16 MHz in this simulation profile |

## Firmware artifact

The example's AVR-GCC ELF runs the ATmega1281 reset path, enables USART0 transmission,
reports STK600 1281 READY, and follows SW0 on LED0. The simulator captures USART0 output at
the MCU boundary; the output does not represent a physical serial bridge.

## Pins and jumper harness

The STK600 has separate LEDS and SWITCHES headers that the user routes to an AVR port.
The following pairs are the explicit demo harness:

| STK600 label | ATmega1281 pin | Polarity |
| --- | --- | --- |
| LED0 | PB5 | Active-low |
| SW0 | PB0 | Active-low |

Install RC09 and SC02 for the ATmega1281 TQFP64 package. Jumper LED0 to PB5 and SW0 to PB0.
The STK600 LED lights when PB5 is low. SW0 reads low while pressed and returns high when
released. These are selected demo wires, not fixed PCB routes.

## Support matrix

| Capability | Status | Boundary |
| --- | --- | --- |
| ATmega1281 CPU and reset path | ✅ | Reuses existing chip implementation |
| GPIO PB0/PB5 behavior | ✅ | Digital pin stimulus and output state; no analog electrical model |
| USART0 capture | ✅ | Simulator-side serial sink |
| STK600 programmer and routing cards | ❌ | Not simulated |
| USB or RS-232 bridge | ❌ | No bridge or connector routing claim |
| Physical clock source, timing, signal integrity | ❌ | Simulation uses fixed 16 MHz |
| Board-level silicon capture | ❌ | Not performed |

## What this catches and what needs a bench

The smoke catches startup, USART0 output, GPIO direction, active-low switch polarity, and
the relationship between the switch and LED output. A physical bench is still needed for
socket-card fit, jumper correctness, clock selection, electrical behavior, serial cabling,
and board acceptance.

## How to run

From the Core repository, build the fixture and run its CI smoke:

    examples/stk600-atmega1281/ci/build.sh
    examples/stk600-atmega1281/ci/test.sh

The runnable CLI script is [io-smoke.yaml](../../examples/stk600-atmega1281/io-smoke.yaml).
The Playground builder uses the committed ELF and does not compile this AVR source.

## Related systems and examples

- [Bare ATmega1281 system](../../configs/systems/atmega1281.yaml) and
  [generic chip smoke](../../examples/atmega1281-io/README.md) remain unchanged.
- [Board onboarding playbook](../board_onboarding_playbook.md)
- [Required source documents](../../examples/stk600-atmega1281/REQUIRED_DOCS.md)
- [External component boundary](../../examples/stk600-atmega1281/EXTERNAL_COMPONENTS.md)
