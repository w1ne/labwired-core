# Final numerical guard integration

The source at `30e28feb8f43bb65ed8e1d643fdcc42852a9a217` combines the reviewed
initializer, voltage-source group treatment, exact contact boundary rule, and
OPAMP inactive numerical-zero guard. The guard retains raw solved voltages and
does not change activation, overvoltage limits, global convergence tolerances,
or the physical circuit.

The 33 focused tests pass. Both unchanged r4 source-bound circuit models accept
initialization and one 100 ns step. Every non-default controller input is an
explicit zero-voltage or disabled-switch stimulus. No MCU or firmware executes.
The first combined step is expensive; this proof establishes no interactive or
long-run performance bound.

| Circuit | Physical / initial unknowns | Selected limit | Constructor | First 100 ns | Peak RSS |
|---|---:|---:|---:|---:|---:|
| fc | 214 / 232 | 512 | 0.004534 s | 0.013891 s | 4224 KiB |
| five_mcu | 666 / 736 | 737 | 0.088962 s | 4.326407 s | 12928 KiB |

`summary.json` binds source identity, exact input hashes, costs, and scope.
The raw JSON files retain all solved samples and recorded input stimuli.
Compressed model fixtures preserve the exact tested JSON bytes; their embedded
netlists remain unchanged. The public-API probe and original owned runner are
included for reconstruction. The runner records the original build and fixture
paths; a new checkout must resolve its own compiled library and decompressed
fixture paths.

Earlier operating-point and first-step refusals remain in
`../analog-initial-supernode-20261112`. The separate r5 mux exports and actual
native firmware probes are not represented as accepted by this packet.
