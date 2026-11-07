# iCE40 UltraPlus MDP Renode scope

Renode's board file loads the Fomu VexRiscv profile. Its CPU is configured as
`rv32im_zicsr_zifencei`, with an 8 KiB ROM at `0x0` and 128 KiB SRAM at
`0x10000000` ([board REPL](https://github.com/renode/renode/blob/master/platforms/boards/ice40up5k-mdp-evn.repl),
[CPU REPL](https://github.com/renode/renode/blob/master/platforms/cpus/fomu.repl)).

LabWired's imported-system profile is narrowed to the generic RV32IM integer
subset, that ROM window, and SRAM. The direct-ELF smoke executed 100,000
instructions, stored `17 * 19` (`0x143`) at `0x10000000`, and passed both
assertions. It made zero peripheral accesses. The run assumes an ideal power
rail.

This result does not exercise reset-vector boot, Zicsr or Zifencei, SPI flash,
I2C, sensors, or board I/O. The complete board remains unverified. Build and run
logs, the simulator result, JUnit assertions, and snapshot are in
`docs/coverage/renode-onboarding/metrics/ice40up5k-mdp-evn/`.
