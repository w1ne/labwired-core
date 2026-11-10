# Hosted microSD verification scope

## Earlier initialization smoke

The earlier low-level SPI initialization sketch returned
`status: model_verified` from the hosted `labwired_verify` service. Its diagram attached
the microSD device to GPIO18/19/23 with CS=GPIO5 and 3V3/GND; the oracle checked
GPIO16 high after the firmware saw the expected CMD0, CMD8/R7, CMD55, and ACMD41
responses. This evidence applies only to that initialization sketch. It also
reported the usual approximation that VDD was assumed to be an ideal 3.3 V
rail.

## Current filesystem smoke

The hosted verification service was also run with the current Arduino SD
firmware, the same wiring, and a GPIO16-high oracle for a successful file read.
It returned `status: failed`, `run_status: pass`, `stop_reason: max_steps` at
225,000,000 steps, with GPIO16 low. The verifier's diagnosis said the program
was still running; the GPIO oracle did not pass.

The hosted catalog currently describes `microsd` as an SPI CMD shell with
CMD17 zero-filled reads and no filesystem. The repository-local model has since
gained a fixed read-only FAT16 image and is the source of the committed
filesystem-read smoke pass. Hosted verification does not validate that local
filesystem behavior. Neither result is physical hardware evidence.
