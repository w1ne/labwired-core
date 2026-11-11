# NUCLEO-H753ZI validation

`io-smoke.yaml` builds `firmware-nucleo-h753zi-smoke` for Cortex-M7 and runs
it against the exact `stm32h753zi` chip descriptor and `nucleo-h753zi` board
manifest. The smoke checks the USART3 register console marker, LD1's PB0 GPIO
edges, and the expected bounded stop. Its build and simulation logs are
committed under `evidence/logs/`; the machine-readable run result is under
`evidence/simulation-output/`.

The console assertion checks the modeled USART3 block. It does not simulate
the board's ST-LINK VCP bridge or validate the PD8/PD9 voltage waveform. The
GPIO assertion checks the modeled GPIOB output state. No silicon comparison
or physical-board acceptance is claimed.

Other board LEDs, Ethernet, USB, ST-LINK, the complete Nucleo-144 header map,
and H753 peripherals outside the descriptor remain unsupported by this
profile. The machine-imported `configs/chips/onboarding/stm32h753.yaml` is not
used by this smoke; its zero-answering imported blocks do not count as
modeled hardware.
