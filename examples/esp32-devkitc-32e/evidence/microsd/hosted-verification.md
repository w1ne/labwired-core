# Hosted microSD verification scope

## Earlier initialization smoke

The earlier low-level SPI initialization sketch returned
`status: model_verified` from the hosted `labwired_verify` service. Its diagram attached
the microSD device to GPIO18/19/23 with CS=GPIO5 and 3V3/GND; the oracle checked
GPIO16 high after the firmware saw the expected CMD0, CMD8/R7, CMD55, and ACMD41
responses. This evidence applies only to that initialization sketch. It also
reported the usual approximation that VDD was assumed to be an ideal 3.3 V
rail.

## Current filesystem write/read smoke

The hosted verification service was run again with the Arduino SD firmware
that reads `/README.TXT`, creates `/ROUND54.TXT`, writes a marker, and reads it
back. With the same wiring and GPIO16-high oracle, it returned `status: failed`,
`run_status: pass`, `stop_reason: max_steps` at 225,000,000 steps, with GPIO16
low. The verifier's diagnosis said the program was still running; the GPIO
oracle did not pass.

The hosted catalog currently describes `microsd` as an SPI CMD shell with
CMD17 zero-filled reads and no filesystem. The repository-local model has a
seeded FAT16 image and single-block writes held for one run; it is the source
of the committed filesystem smoke pass. Hosted verification does not validate
that local filesystem behavior. Neither result is physical hardware evidence.
