# RC Oscilloscope Lab

An STM32F401 drives a first-order RC low-pass (R1 = 10k, C1 = 100n, tau = 1 ms)
from PA5 and samples the filtered node on ADC1 channel 0 (PA0). PA5 steps
every 5 ms, the ADC is read every 500 us, and each sample is printed over
USART2 as `t=<us> adc=<code> v=<mV>`. The RC network is not a device model:
it is a SPICE netlist, inline in `system.yaml`, run by the in-core analog engine through the
`cosim_models` block in `system.yaml`, and the playground opens the lab with
the oscilloscope instrument showing `v(out)` charging and discharging with
each PA5 edge.

## Files

- `system.yaml` — STM32F401 chip, `debug_uart: uart2`, `board_io` for the PA5
  drive (`led`) and the PA0 sample point (`adc_input`), and the `cosim_models`
  entry with the inline netlist (`netlist_text`). `Vgpio` is the source the
  engine drives from PA5. The netlist is inline so the same manifest runs in
  the browser, which has no filesystem.
- `src/main.rs` — bare-register firmware. TIM2 is the 1 MHz tick that
  schedules both the 5 ms toggle and the 500 us sample, so the timing is
  exact under simulation rather than busy-wait approximate.
- `test.yaml` — smoke test asserting the banner, the sample lines,
  `tau_us=` in 800..1200 and `rc_shape=ok`.

## Printed verdicts

After every PA5 rising edge the firmware reads the node at the edge, then
watches each 500 us sample until it crosses 63.2 % of VDD (2085 mV). It
prints the crossing time, linearly interpolated between the two samples
that bracket it, as `tau_us=<n>`, followed by `rc_shape=ok` when the
readings from the edge to the crossing were monotonic non-decreasing and the
crossing fell in 800..1200 us; otherwise `rc_shape=bad`. A flat or slowly
ramping ADC never produces `rc_shape=ok`, which is what makes the test a
check of the analog loop rather than of the firmware's print path.

## Build the firmware

```
rustup target add thumbv7em-none-eabi
cargo build -p rc-oscilloscope-lab --release --target thumbv7em-none-eabi
```

The ELF lands at `target/thumbv7em-none-eabi/release/rc-oscilloscope-lab`.
Example ELFs are not committed; the playground fetches prebuilt demo firmware
from the `firmware-demos-v1` GitHub release (see
`packages/playground/scripts/fetch-demo-firmware.sh` in the labwired
monorepo), so this ELF needs to be added to that release when the lab ships.

## Run natively

```
labwired test --script examples/rc-oscilloscope-lab/test.yaml --analog-trace out.csv
```

`--analog-trace` writes the probed `v_out` samples to CSV for plotting or for
the validation matrix golden file.

## Verified result

With the in-core `analog` adapter the test passes 5 of 5 checks. Every PA5
rising edge prints `tau_us` between 998 and 1006 and `rc_shape=ok`, and
`--analog-trace` writes one row per 100 us model step.
