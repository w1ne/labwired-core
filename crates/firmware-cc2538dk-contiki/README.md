# firmware-cc2538dk-contiki

Reproducer for the **unmodified upstream Contiki-NG v4.9** fixture ELF that the
`firmware_survival` gate and `examples/cc2538dk/contiki-hello.yaml` boot on the
TI **CC2538SF53** (CC2538DK: CC2538EM on a SmartRF06 EB, Cortex-M3):

| fixture (`tests/fixtures/`)    | Contiki-NG example     | what it proves |
|--------------------------------|------------------------|----------------|
| `cc2538dk-contiki-hello.elf`   | `examples/hello-world` | stock Contiki-NG boots through the CC2538 boot ROM, SYS_CTRL clock switch, RF core bring-up (random seed, CSMA radio init) and the sleep-timer count, then prints `Hello, world` over UART0 every 10 s |
| `cc2538dk-contiki-test-cfs.elf` | `examples/storage/cfs-coffee` `test-cfs` | a 65 000-byte Coffee file written, read back and appended through ROM `PageErase`/`ProgramFlash` and the flash controller |
| `cc2538dk-contiki-coffee.elf`  | `examples/storage/cfs-coffee` `example-coffee` | Coffee file write, read, append and directory listing |
| `cc2538dk-contiki-flash-test.elf` | `flash-test/` (this directory, MIT) | the steps of Renode's `cc2538_flash_controller.robot` on the ROM utility API |
| `cc2538dk-contiki-crypto-<t>.elf`, t = `ecb` `cbc` `ctr` `cbc-mac` `gcm` `ccm` `sha256` | `examples/platform-specific/cc2538-common/crypto` `<t>-test` | every vector of the AES/SHA-256 suite through the key store, the cryptoprocessor DMA and the AES interrupt; cbc/cbc-mac/ctr/ccm print the lines Renode's `cc2538_cryptoprocessor.robot` waits for |

This is the binary the Renode `cc2538_single-node.robot` test runs (Renode
fetches a prebuilt one and loads a TI ROM dump; LabWired runs its own
clean-room ROM, `crates/core/roms/cc2538/`).

The upstream firmware is not patched (`flash-test/` is LabWired's own
application). hello-world exercises:

- the clean-room ROM: SWRU333A §2.2 CCA boot (VTOR = `0x0020_2000`) and the
  utility API at `0x48` (`.data` copy and `.bss` clear in `reset_handler`);
- `SYS_CTRL_CLOCK_STA.SYNC_32K` (`sys_ctrl_init` waits for a 32-kHz edge);
- the RF core (`random_init`: `ISRXON`, `RSSI_VALID`, `RFRND`; the CSMA radio
  driver init: channel, CCA, MAC timer);
- the flash information page's primary IEEE address
  (`Link-layer address: 0012.4b00.0000.0001`);
- the sleep-timer count (rtimer, LPM duration checks) and SysTick (the 10-s
  etimer). It never arms a sleep-timer compare: CSMA keeps the radio
  listening, so LPM stays in PM0.

## Build

```sh
./build.sh          # -> tests/fixtures/cc2538dk-contiki-{hello,test-cfs,coffee,flash-test,crypto-*}.elf
```

Requirements: `arm-none-eabi-gcc` and git. The script clones Contiki-NG
release/v4.9 into `$CONTIKI_NG` (default `~/ext/contiki-ng-src/contiki-ng`) if
it is not there. Contiki-NG is BSD-3-Clause.

This directory is a build wrapper only; it is not a Cargo crate.
