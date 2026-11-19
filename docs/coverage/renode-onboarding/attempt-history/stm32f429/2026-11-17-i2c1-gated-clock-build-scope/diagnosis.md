# I2C1 smoke build scope error

The target-only runner stopped at `build_firmware`: the initial implementation declared the `gated` and `clock` result variables inside an `unsafe` block, then used them outside it. This is a firmware compile failure before simulation; no run was attempted. The compiler log and metrics are retained here. The values are now returned from that block as a tuple.
