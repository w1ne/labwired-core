# Actual CODAL core-reset qualification lane

This proposed slice is not a passing guest result or complete board reset.
The actual caller is pinned to Runtime
`12d4759af0bf2b29726cfd79c487474a282cc5b0`; this revised source needs a new
twenty-mode build audit. The predecessor's build passed with all old0–18 ELF
hashes unchanged, but does not qualify this new image or the engine correction.

The first actual [run 38698483581](https://github.com/CrispStrobe/labwired-core/actions/runs/38698483581)
failed before any reset request: the probe incorrectly expected the Thumb bit
from `MRS xPSR`. Arm's table 2-3 specifies EPSR reads zero through this
instruction. The revised caller checks that software-visible value separately
from the unchanged full debugger xPSR check. Keep the original failure.

That original also exposed missing RAM halfword/word callbacks on the fast bus
path. The revised probe explicitly enables
`SystemBus.config.observe_wide_ram_stores` (default false). Only committed RAM
stores report actual pre-store and post-store bytes; flash/MMIO and existing
default observer behavior are unchanged. Permanent unit controls check opt-in,
same-valued stores, existing partial byte-fallback faults and opt-out. Do not substitute invented
byte events or read the bus through an aliased observer reference. This is
diagnostic instrumentation, not a measured performance optimization.

The retained first run also had one CPU batch per instruction at quantum64:
the complete board manifest's NeoPixel resident requires per-cycle service.
The new reset-only fixture explicitly clears external board devices in both
quantums, retains the chip/core models, and reports that fixture identity and
zero external-device count. It still requires actual multi-instruction windows;
it does not claim batching or reset of the fully wired board. All nineteen
earlier caller fixtures keep their unchanged board setup. Full-board resident
timing and reset qualification remain separate work, not a reason to relax the
reset-only fixture's batching assertion.

The focused `CortexM::reset` correction clears prior masks, IT/PSR and LR
according to [Arm's Cortex-M4 register summary](https://documentation-service.arm.com/static/5f2ac4ab60a93e65927bbdbf).
This was a source-review finding, not a claimed earlier guest failure. Permanent
unit controls deliberately poison the prior state. The actual target_reset
caller separately requires pre-CPSID startup register capture, real invalid-key
return versus valid AIRCR store/reset cut, two startup/data/BSS epochs, retained
NOLOAD witness, pre-reset core-state readbacks and closed post-terminal bounds.
Use actual retired store opcode/registers, not invented SCS write callbacks.
Previous-retirement registers at reset entry are OLD context; only the actual
post-first-LDR snapshot and startup's own register stores describe the new boot.

Because image loading also invokes CPU reset, the hosted workflow clean-builds
and repeats all nineteen earlier service/heap/SPI modes at unchanged Runtime
`932d476344107386d56dfd2d595d74d713780563`, under both scheduler profiles.
The new caller also runs single-step and real bounded-batch paths in both.
Every profile/caller executes despite another failure; retain all originals.
Do not merge on unit or build evidence alone. Freeze independent source/image/
provider predicates and refusal controls before sole original acquisition;
then require every enabled final-head check and exact tested/landed tree proof.

The selected engine reset is core-local: peripheral models, external pending
sources, flash commands/persistence, USB/QSPI, physical reset behavior, complete
board/API/build/image admission, native Arcade, active RTx and installed consumer
qualification remain separate. The [board/API continuation](samd-runtime-replacement-plan.md#current-continuation-after-actual-spi-callers)
and [landed lifetime checkpoint](samd-real-spi-lifetime-qualification.md) remain
authoritative. No consumer pin, ACK, physical capture or threshold change.
