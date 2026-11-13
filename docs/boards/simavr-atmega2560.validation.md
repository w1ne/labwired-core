# Imported ATmega2560 onboarding smoke

The imported system profile is bounded to the ATmega2560's 256 KiB flash,
8 KiB SRAM, GPIO ports B/E/F/G/H, USART0, and ADC inputs. Its fixture prints a
UART banner and ADC readings for channels 0, 8, and 9; the smoke checks those
values, the D13/PB7 output edges, and the configured instruction limit. The
other configured GPIO pins are not asserted. SPI, TWI, other USARTs, and other
board peripherals are outside this result.

The first 3,000,000-step attempt reached the `k=1` ADC report before PB7's
scheduled LED transition and also exposed unmapped writes to GPIO ports E/F/G/H.
The measured profile maps those fixture-used ports and runs for 15,000,000
steps; the final result has no unmapped-MMIO fidelity entries and observes the
PB7 edges.

The run assumes an ideal power rail; it does not exercise power-on or
brown-out reset behavior.

Evidence, including the result, UART output, snapshot, JUnit report and build/run
logs, is in
[`docs/coverage/renode-onboarding/metrics/simavr-atmega2560/`](../coverage/renode-onboarding/metrics/simavr-atmega2560/).

This is a local LabWired CLI smoke against the imported manifest. It does not
validate the whole ATmega2560 platform, Renode equivalence or physical hardware.
