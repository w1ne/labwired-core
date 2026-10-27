# ATmega1281 on STK600

This Playground target uses the ATmega1281 chip model with a separate STK600 board
manifest. The official STK600 device table lists the ATmega1281 with routing card RC09
and socket card SC02 for the TQFP64 package.

The Playground demo harness models two explicit jumper wires:

| STK600 header signal | ATmega1281 pin | Polarity | Demo behavior |
| --- | --- | --- | --- |
| LED0 | PB5 | Active-low | Turns on while SW0 is pressed |
| SW0 | PB0 | Active-low | Press reads low; release reads high |

The STK600 headers are routed with jumper cables, so the mapping above is this example's
harness definition, not a fixed board connection. For a physical setup, fit RC09 and SC02,
then jumper LED0 to PB5 and SW0 to PB0. The LED lights when PB5 sinks current; the switch
pulls its line low when pressed and has a pull-up when released.

The smoke firmware sets the simulator clock to 16 MHz, prints STK600 1281 READY on
USART0, and mirrors the SW0 level to LED0. The simulator captures USART0 output directly
from the simulated MCU. This does not model or claim an STK600 USB or RS-232 bridge.

From core, build the committed AVR-GCC fixture and run the smoke:

    examples/stk600-atmega1281/ci/build.sh
    examples/stk600-atmega1281/ci/test.sh

The example runs a committed AVR-GCC ELF; the Playground builder does not compile this C
source. Rebuild the ELF with AVR-GCC after changing src/main.c.

## Model boundary

The board system reuses the existing atmega1281 chip descriptor and simulator. It models
only the named GPIO harness plus simulator-side USART0 capture. It does not emulate the
STK600 programmer, socket-card routing electronics, selectable physical clock circuitry,
USB, RS-232, or other headers and peripherals. The 16 MHz clock is a fixed simulation
setting. No physical-board, electrical, timing, or silicon-capture validation is claimed.

See REQUIRED_DOCS.md, EXTERNAL_COMPONENTS.md, and VALIDATION.md for source references and
evidence.
