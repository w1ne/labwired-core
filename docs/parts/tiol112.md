# TIOL112 IO-Link digital transceiver

The model follows TI [SLLSFJ1D](https://www.ti.com/lit/ds/symlink/tiol112.pdf)
for the digital TX/RX/CQ truth table and EN control. It is a distinct kit
(`tiol112`), not an alias for MAX485 or a generic IO-Link drawing.

| Aspect | Implementation |
|---|---|
| Model and kit | `crates/core/src/peripherals/components/tiol112.rs` |
| Attachment | Modern STM32 USART plus GPIO EN, WAKE and NFAULT |
| Example | `examples/iolink-g0b1re-tiol112/` |
| Coverage | Digital behavior; timed UART propagation when a timed link is attached |

## Attachment

```yaml
external_devices:
  - id: phy
    type: tiol112
    connection: usart1
    config:
      en: PA8
      wake: PB0
      nfault: PB1
      driver_delay_ns: 1200
      receiver_delay_ns: 300
      enable_delay_ns: 4000
      disable_delay_ns: 4000
```

`en` is required and accepts a GPIO pad or `high`/`low`. Unconnected EN is
represented explicitly by `low`. `wake` and `nfault` are optional GPIO pads.
Unknown keys, unknown pads and out-of-range propagation delays fail during
attachment. The delays default to the datasheet maxima; the configurable
ranges are 0..1200 ns TX-to-CQ and 0..300 ns CQ-to-RX. Enable and disable
delays are separate configurable scenarios; both default to the datasheet's
4 µs **typical** value, which is not a guaranteed maximum. A UART character
that starts before the configured enable time is rejected, rather than
converted into an artificial complete start bit.

| TI pin | Role | Typical G0B1 reference connection |
|---|---|---|
|1 VCC_IN | Logic supply, TIOL112 (TIOL1123 has a different pin1 function) |3.3V |
|2 NFAULT | Active-low open-drain status | PB1, pull-up |
|3 RX | Inverted CQ receiver output | PA10 / USART1_RX AF1 |
|4 TX | Transmitter input | PA9 / USART1_TX AF1 |
|5 EN | High enables CQ driver | PA8 |
|6 ILIM_ADJ | Analog current-limit setting | Datasheet resistor network; unmodeled |
|7 L- | Return | Common ground |
|8 CQ | IO-Link port | Virtual master CQ |
|9 L+ | Port supply |24V connection; analog supply unmodeled |
|10 WAKE | Active-low open-drain wake indication | PB0 / EXTI0, pull-up |

## Observable behavior

| Condition | Result |
|---|---|
| EN low, release delay elapsed | CQ driver high impedance; MCU bytes do not reach the master |
| EN high, enable delay elapsed, TX low | CQ high |
| EN high, enable delay elapsed, TX high | CQ low |
| CQ low / high | RX high / low |
| MCU receiver enabled during TX | Local CQ echo reaches RDR |
| EN falls before TC | Character truncates if configured driver release precedes its stop end |
| Master and MCU drive together | No clean master byte is delivered |
| `wake=1` stimulus | WAKE goes low on next peripheral service, reaches normal GPIO/EXTI path |
| `fault=1` stimulus | NFAULT goes low; protection physics is not synthesized |

In timed mode `Tiol112::tx_frames()` exposes the actual admitted character's
first CQ start time, stop end, physical polarity and truncation. The underlying
UART samples start/data/parity/stop bits and models TXE/TC/RXNE/ORE. Endpoint
propagation is separate from configurable cable delay.
The network transports the recovered UART polarity; `CqFrame::cq_level_at`
exposes the corresponding inverted physical CQ logic level.

On STM32G0, attaching this PHY also creates a live RCC clock contract. A
configured 64 MHz operating timebase requires firmware to switch to a matching
PLLR clock before SDCI traffic. Divided APB, unsupported UART clock sources and
nonzero USART PRESC are rejected. An early disabled UART does not produce a
clock error. Model logs expose clock rejection and premature-enable counters.

`Tiol112::set_wake_asserted(true)` and the `wake` simulator input share the
same logical stimulus. Set either to false to release WAKE. The signal reaches
the normal GPIO/EXTI route on the next resident-device service.

The ordinary `iolink-master` stream remains a functional fixture. Legacy
sessions use tick pacing; the explicit `tiol112-eval` profile uses elapsed-time
byte pacing, a Start action, logical WAKE and validated DPP bring-up. Neither
stream mode models a physical reply stop/start waveform. Use the native timed
test for timing evidence:

```bash
cargo test -p labwired-core --test g0_tiol112 --test modern_uart_timing
```

## Fidelity limits

The 4 µs enable/disable figures are modeled as explicit typical scenarios;
they do not provide a silicon worst-case guarantee. Analog thresholds, drive current,
short-circuit/thermal protection, loading, reflections, EMC, or the physical
current pulse that produces WAKE. Pull-ups are ideal. WAKE/NFAULT are explicit
stimuli. The SIO GPIO-level waveform is not joined to the timed USART path.
These limits prevent digital test results from becoming electrical conformance
or worst-case response guarantees.

Related: [STM32G0B1RE](../boards/stm32g0b1re.md),
[full rig validation](../../examples/iolink-g0b1re-tiol112/VALIDATION.md).
