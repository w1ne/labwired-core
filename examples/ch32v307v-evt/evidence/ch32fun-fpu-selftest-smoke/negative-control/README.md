# Negative control: the FPU self-test on a core without F

The same `ch32fun/fpu_selftest.elf` and system as `ch32fun-fpu-selftest-smoke.yaml`,
with the chip descriptor's single line `core: qingke-v4f` changed to
`core: qingke-v4b` (the CH32V203's RV32IMAC QingKe V4). The copies lived in a
temporary directory, shown here as `negative-control/chip.yaml`, `sys.yaml`
and `smoke.yaml`.

Result: FAIL 0/6. After 114 instructions the core stops with
`decode_error` at 0x79e, on `0x12812e27`, an `fsw`: the first FP instruction
on ch32fun's path to `main`. Without the F extension, nothing in the
firmware's output appears.
