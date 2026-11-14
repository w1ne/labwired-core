# STM32U5 IO-Link firmware prerequisites

The customer `nucleo_u575zi_q` Zephyr reference ELF runs its own startup,
initializes TIM2 with prescaler 159, and measures time at one microsecond per
160 simulated CPU cycles. PB1 falling-edge WAKE reaches the actual Zephyr EXTI1
handler, clears FPR1, and causes the firmware to configure USART2 at COM2
(BRR 4167, 8 data bits, even parity, one stop bit). PD5 changes from SIO GPIO
output to USART2 AF7; PD6 also selects USART2 AF7. The adapter error diagnostic
remains zero. These assertions run in both legacy and event-scheduler modes.

The EXTI model follows the U575 register map: EXTI owns the port mux at 0x60,
RPR1/FPR1 are independent W1C pending registers, and GPIO lines 0..15 route to
individual NVIC IRQs 11..26. Four-bit mux slots include GPIOI. Focused tests
cover wrong-port rejection, masked-edge latching and selective byte W1C.
Sources are [ST RM0456](https://www.st.com/resource/en/reference_manual/rm0456-stm32u5-series-armbased-32bit-mcus-stmicroelectronics.pdf)
and `stm32u575xx.h` from Zephyr's pinned `hal_stm32` revision
`0657d9f97d973542e438f628a704f5d8ea0cdef5`.

This validates a firmware prerequisite, not an electrical IO-Link Twin. C/Q
voltage/current, transceiver propagation, cable loading, supply faults, master
exchange and a physical board remain unverified. The GPIO EXTI model does not
implement software/internal triggers, EMR event delivery, or security,
privilege and locking enforcement. Existing timer limits still apply; this
firmware uses a fixed 160 MHz clock and APB1 prescaler one.

Build the [customer source example](https://github.com/w1ne/iolinki/tree/5572a628b8befa6a9c74496311da2359868f96d9/samples/stm32u5_tiol112)
with Zephyr `c66235fb7346bbe3dbedd1dd76ec5a37a8e8262b` and that revision's
CMSIS/STM32 modules. `west init --mr` accepts a branch or tag, not this
commit, so fetch the commit and run `west init -l` on that checkout, then
`west update cmsis hal_stm32`. Then run:

```sh
export IOLINKI_U5_ELF=/absolute/path/to/zephyr.elf
cargo test -p labwired-core --test stm32 -- --include-ignored u5_exti:: u5_reference_firmware::
cargo test -p labwired-core --features event-scheduler --test stm32 -- --include-ignored u5_exti:: u5_reference_firmware::
```

The actual-ELF test is ignored in ordinary host runs because it requires an
external board build; explicitly running it without the environment variable
fails. The IO-Link prerequisite CI workflow builds these pinned sources,
requires both execution modes and archives the ELF/map/HEX with an ELF SHA-256.
Local validation used ELF SHA-256
`15250b572d9139656ab8772883a78a7b0fa2b1265635212a8605ed6f667158e7`.
