# Validation — Curiosity PIC32MZ EF 2.0 (PIC32MZ2048EFM144)

Claims, evidence and what is explicitly **not** claimed. **SIM-DERIVED**: there
is no PIC32MZ bench part, and no Renode or QEMU run was diffed against (QEMU's
MIPS targets are Malta/Boston boards with no PIC32MZ peripherals; Renode has
no PIC32).

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| MIPS32r2 integer ISA, little-endian: ALU/shift/rotate, MULT/DIV, MUL/MADD/MSUB/CLZ/CLO, EXT/INS/WSBH/SEB/SEH, RDHWR | unit + firmware | `mips_tests` (encodings from `mips-zephyr-elf-as -EL -march=mips32r2`); both NuttX images |
| Branch delay slots, branch-likely nullification, linking branches and jumps | unit + firmware | `delay_slots_branch_likely_and_link`; every image |
| LWL/LWR/SWL/SWR (little-endian), LL/SC with ERET clearing LLbit | unit + firmware | `unaligned_word_load_and_store_are_little_endian`, `ll_sc_and_eret_clears_the_link`; NuttX `memcpy`/`strcpy` |
| Exceptions: overflow, syscall/break, reserved instruction, address error (incl. user access to kseg), TLB refill for every mapped address, coprocessor unusable; EPC/Cause.BD in delay slots; BEV/EBase/IV vectors | unit | `addi_overflow_traps_and_leaves_rt`, `exception_in_a_delay_slot_sets_bd_and_points_at_the_branch`, `misaligned_and_mapped_accesses_fault`, `wait_idles_until_a_request_and_tlb_ops_are_reserved` |
| Shadow register sets (8, HSS = 7): RDPGPR/WRPGPR, ESS on exceptions, PRISS on interrupts, PSS on ERET | unit + firmware | `shadow_register_sets_through_rdpgpr_wrpgpr`, `evic_priority_masking_and_multi_vector_offsets`; NuttX's head.S writes GP into every set |
| EVIC in EIC mode: IFS/IEC/IPC with CLR/SET/INV, priority vs Status.IPL, subpriority, INTSTAT, single vector (EBase + 0x200) and multi vector (EBase + OFFn) | unit + firmware | `evic_single_vector_interrupt_entry_and_eret`, `evic_priority_masking_and_multi_vector_offsets`; NuttX's `pic32mz_decodeirq` loop on INTSTAT |
| Core timer (Count at SYSCLK/2, Compare match → IRQ 0, Cause.TI) and core software interrupts (Cause.IP0/IP1 → IRQ 1/2, latched on the rising edge) | unit + firmware | `core_timer_match_flags_irq0_and_compare_write_clears_ti`, `core_software_interrupt_latches_on_the_rising_edge`; every NuttX context switch is a software interrupt 0 |
| Timer1 (PBCLK3 / TCKPS, period match → T1 flag, SOSC with TCS) and Type B prescalers | unit + firmware | `pic32_timer` tests; NuttX's 100 Hz tick (`sleep`, ostest timed waits, round robin) |
| UART2: immediate TX with the TX-empty flag, RX threshold flag, character-time receive polling, SET/CLR/INV aliases | unit + firmware | `pic32_uart` tests; `nuttx-nsh.yaml` and `curiosity_pic32mz_nuttx` type commands over U2RX |
| Idle: WAIT, and a proven fixed-point loop (NuttX's `up_idle` spin) fast-forwarded to the next event | unit + firmware | `a_fixed_point_loop_is_idle_but_a_polling_loop_is_not`; the nsh run executes 99,848 instructions in 60 M cycles |
| NuttX nsh (prompt, `hello`, `ps`, `ls /dev`) | firmware | `nuttx-nsh.yaml`, `curiosity_pic32mz_nuttx::nsh_runs_typed_commands`, `firmware_survival::test_curiosity_pic32mz_nuttx_survival` |
| NuttX ostest, 34 groups, 0 errors | firmware (manual) | `nuttx-ostest.yaml` |
| Instruction support for both images | audited | `unsupported_instruction_audit.sh`: 2,000,000 steps each, 0 unsupported; a reserved SPECIAL3 word patched into `nsh_main` reports 1 |
| Negative controls | checked | Core software interrupt as a level instead of an edge: NuttX clears CS0IF before Cause.IP0, the stale flag re-enters the switch and the system reboots in a loop. Timer1 routed to an unused vector: `sleep 1` never returns and the queued `hello` never runs. NuttX 12.11.0 (before upstream 7956039dfd): the Idle task's saved context is overwritten on the first interrupt and the board reboots, as on silicon |
| Determinism | checked | fixtures rebuild byte-identically (`build.sh`, sha256 in its output; serial make, `SOURCE_DATE_EPOCH` pinned) |

## Not claimed

- **FPU**: CP1 instructions raise coprocessor unusable while Status.CU1 is
  clear and are reserved instructions after; the NuttX images are
  soft-float.
- **microMIPS**: an image whose DEVCFG0.BOOTISA selects microMIPS is refused
  at reset; JALX is a reserved instruction.
- **DSP ASE, TLB, EJTAG**: their instructions are reserved; every mapped
  address (KUSEG with ERL clear, KSEG2/3) takes a TLB refill exception.
- **Caches and wait states**: CACHE/PREF/SYNC/SYNCI are no-ops; one
  instruction is one CPU clock.
- **Bus errors**: an access to unmapped space stops the simulation instead of
  raising IBE/DBE.
- **EVIC**: the interrupt proximity timer, external interrupt pins (INTx) and
  persistent-source semantics beyond the UART and timers; a peripheral flags
  IFS on its event.
- **UART2**: baud-accurate TX timing, parity/framing errors, OERR, address
  detect, IrDA and flow control.
- **Ports/PPS, OSC, CFG (SYSKEY), flash controller, WDT, DMT, prefetch**:
  declared stubs. No clock rate follows OSCCON/SPLLCON/PBxDIV: `cpu_hz` is
  the 200 MHz the board's DEVCFG selects, PBCLK2 100 MHz and PBCLK3 50 MHz.
- Timers 2-9, DMA, SPI, I2C, CAN, Ethernet, USB, ADC, crypto, RNG: not
  mapped.
