# UT32M0R500 stimulus schema preflight

The target-only inventory runner rejected the first IRQ smoke attempt before
execution because the fixture declared schema `1.0`; `stimuli` require schema
`1.2`. The runner recorded a `config_error` at 0 instructions. No boot or
smoke result is claimed from this attempt. The metrics, build/run logs and
simulation output are retained here before correcting the fixture schema.
