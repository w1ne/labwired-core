# LabWired hosted verification

- Result: `status: model_verified`; run status `pass`.
- Oracle: serial contains `FEATHER_NRF52840_UART_OK`.
- The oracle stopped on the matching UARTE0 output. The submitted source
  configured P0.25, selected it as UARTE0 TX, set 115200 baud, and sent the
  marker with EasyDMA from RAM.
- The hosted catalog has no Feather nRF52840 Express board entry, so this
  peripheral probe used the hosted `nrf52840-dk` catalog target, which runs the
  nRF52840 MCU model. It verified the same UARTE0 register path, but is not a
  hosted board-level model of the Feather.
- Non-blocking verifier gaps: device time is derived from 64 MHz without PLL
  tracking, and the virtual board assumes an ideal VDD rail because no power net
  was routed.

The exact Rust source could not be compiled by the hosted Rust path: its
generated `cortex-m-rt` project fails to link because the harness enables a
device feature without an interrupt vector table. The hosted oracle therefore
used the equivalent bare-metal Arduino C++ UARTE0 probe saved as
[`hosted-verify-source.ino`](hosted-verify-source.ino); the concise verdict is
in [`labwired-verification.json`](labwired-verification.json). Do not read that
hosted result as verification of the Rust ELF.

The exact committed Rust firmware was also compiled and run by the repository's
local LabWired CLI against the focused Feather system. The committed
`simulation-output/result.json` records both smoke assertions passing.
