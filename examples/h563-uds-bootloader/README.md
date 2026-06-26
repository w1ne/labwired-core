# H563 UDS OTA Bootloader Sim Smoke

End-to-end smoke test for the STM32H563 dual-bank UDS OTA bootloader, driven
entirely on-chip over FDCAN internal loopback.

The firmware (`h563_uds_bootloader_sim.elf`) is built from
`udslib/examples/h563_uds_bootloader/` with `SIM_TESTER=1`. That build links a
baked-in tester (`sim_tester/sim_tester.c`) which, in one firmware image, runs
both the UDS **server** (the bootloader) and a UDS **client** that injects the
full OTA reprogramming sequence into the server over the loopback FDCAN bus:

    DiagnosticSessionControl (0x10 02)
      -> SecurityAccess seed/key (0x27 01 / 0x27 02, AES-128-CMAC, multi-byte key)
      -> RoutineControl EraseMemory (0x31 01 FF00)
      -> RequestDownload (0x34, inactive-bank app base, App-B image size)
      -> TransferData x N (0x36, embedded App-B image)
      -> RequestTransferExit (0x37)
      -> RoutineControl CheckProgrammingDependencies (0x31 01 FF01)
      -> RoutineControl ActivateSoftware (0x31 01 FF02, SWAP_BANK + reset)

The App-B image is embedded as a C byte array (`sim_tester/app_b_image_blob.h`,
generated from `app/app_b_image.bin`). Before the swap the tester first copies
the bootloader (sectors 0-11) into bank 2 so the post-swap bank 1 view still
boots a valid bootloader.

## Running

```
cargo run -p labwired-cli -- test --script examples/h563-uds-bootloader/ota-smoke.yaml \
  --output-dir out/h563-uds-bootloader --no-uart-stdout
```

## Building the firmware

```
UDSLIB_DIR=/path/to/udslib \
  make -C /path/to/udslib/examples/h563_uds_bootloader/bootloader SIM_TESTER=1
```

The ELF is written to that example's `build/` directory; `ota-smoke.yaml`
references it directly via a relative path.

## Asserted milestones (pass)

The smoke verifies the full OTA cycle end-to-end — handshake, download, verify,
bank-swap, reset, and boot into the new app:

- `BL-START` — bootloader entry
- `BL-RECOVERY` — no valid active-bank app, server loop entered
- `BL: UDS server ready`
- `SIM: copy OK` — bootloader copied into the inactive bank
- `BL: routine erase` / `BL: erasing inactive app` — EraseMemory routine ran
- `BL: download armed` — RequestDownload accepted; DiagSession + SecurityAccess
  (multi-frame ISO-TP AES-128-CMAC key) + Erase + RequestDownload all completed
  with correct positive server responses
- `OTA-WRITE-OK` — TransferData wrote the App-B image to the inactive bank
- `OTA-VERIFY-OK` — image CRC check passed (`BL: image check PASS`)
- `OTA-ACTIVATE` / `BL: marking inactive bank pending` / `BL: activate software`
  — ActivateSoftware routine programmed `OPTSR_PRG.SWAP_BANK` and committed it
  with `OPTCR.OPTSTRT`, which the FLASH model applies as a bank-swap + reset
- `BL-JUMP` — the rebooted bootloader validated the swapped bank and jumped
- `APP-B v2` — the freshly programmed application is running
