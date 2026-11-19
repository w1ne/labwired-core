# Control: timer outputs not wired to the ports

Same committed ELF and the same Rust test
(`cargo test -p labwired-core --test avr avr_nano_every_pwm::`). The only
change is one line of `crates/core/src/bus/from_config.rs`: the call
`bus.wire_megaavr_timer_output_pads()` is commented out, so no PORT carries a
TCA0 / TCB output line. The line was restored after this run.

Result: the test FAILS as required (`cargo-test.log`). The run stops at the
first `analogWrite()` with `TCA0 compare outputs (CTRLB.CMPnEN) with no port
to carry them is not modelled`. The timer model refuses the output instead of
counting without a pad, and the passing run therefore depends on the pad
wiring.
