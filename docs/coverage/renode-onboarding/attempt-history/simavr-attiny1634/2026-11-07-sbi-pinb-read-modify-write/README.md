# ATtiny1634 GPIO smoke — first attempt failed (SBI on PINB)

First run of the new `simavr-attiny1634` GPIO smoke, after the CPU model gained
ATtiny1634 port mirroring and PINx write-one-to-toggle (data 0x27/0x2B/0x2F),
but before SBI/CBI learned the ATtiny1634 PINx addresses.

- Run: `scripts/run_renode_onboarding_inventory.py --only simavr-attiny1634`,
  100,000 instructions, stop reason `max_steps`.
- Result: **FAIL 2/3**. `memory_value @0x130` expected `0x7F`, got `0x6F`.
  The PB0 edge and stop-reason assertions passed.
- Missing bit `0x10` is the "SBI on PINB toggles PB0 only" check. SBI was
  executed as a read-modify-write of PINB: with PB3 driven high it wrote
  `0x09` back to PINB, which toggled PB3 as well as PB0.
- The new unit test `attiny1634_ports_mirror_and_pin_toggle` failed at the
  same point: PORTB was `0x01` after `SBI PINB,0`, expected `0x09`.

Cause: `write_one_register` in `crates/core/src/cpu/avr/exec/bitops.rs`
fell back to `data_addr == 0x36` for every toggle-capable model not named in
its match, so the ATtiny1634's PINB at 0x2B was not treated as write-one.
The fix gives the ATtiny1634 its own arm and makes the fallback `false`.

`junit.xml` and `run_smoke.log` here are copied from that failed run.
