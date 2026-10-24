# SAMD51 DMAC byte foundation

Status: implementation lane claimed after the qualified
[split-IRQ checkpoint](../receipts/2026-10-12-samd-sercom-split-irq/README.md).
The opt-in candidate implementation and tests are authored; hosted correctness,
actual guest qualification and merge remain pending. No passing DMA result is
claimed from source declarations or local formatting.

## Selected first boundary

Author an MIT, explicitly opt-in `sam_dmac_byte` model from primary register
facts. Do not copy ASF4 implementations or import vendor-restricted headers.
Do not change the production PyBadge descriptor, application pins or captures.
Reserve `sam_dmac` with an explicit not-admitted refusal; it must not silently
fall through the generic name heuristic to an unrelated STM32 DMA register map.

Primary input: Microchip
[DS60001507F](https://ww1.microchip.com/downloads/en/DeviceDoc/SAM_D5xE5x_Family_Data_Sheet_DS60001507F.pdf),
sections22.6.2.3/22.6.2.7/22.6.2.9/22.7/22.8/22.10. For SAMD51, channel
windows begin at0x40 with0x10 stride; BASEADDR/WRBADDR are0x34/0x38 and
16-byte aligned. Descriptors are16 bytes with BTCTRL/BTCNT/SRCADDR/DSTADDR/
DESCADDR. Channels0..3 use IRQ31..34; channels4..31 share IRQ35.

The first slice permits software-triggered BLOCK action, single-byte beats,
single unlinked descriptors, fixed or unit-increment addresses, and NOACT/INT
block actions. Fetch descriptors through the real bus. Observe each byte's
read/write result before decrementing the count. On transfer bus error, retain
the failed beat's count, write back progress, disable the channel and expose
TERR without TCMPL. Clear VALID in final/error writeback. Completion interrupt
depends on the selected block action and enabled interrupt, not host injection.

Mapped descriptor, data and writeback targets are limited to primary SystemBus
RAM. Flash, extra mappings, MMIO (including self/stub/clock-gated windows) and
nonsecure peripheral aliases explicitly refuse before the affected beat;
unmapped targets still exercise actual bus faults. Check all16 bytes of each
table extent before data transfer. A generic bus `Ok` from a dropped MMIO store
cannot establish RAM writeback or DMA completion. Peripheral DMA admission is
a separate follow-on, not implied by this profile.

Require actual authored CPU guests to create their own descriptors, program
DMAC/NVIC, transfer memory, acknowledge the correct handler and return. Test
all five IRQ routes and channel31; masks, no trigger, disabled controller/channel,
reset, read/write bus failures and no success after a failed beat. Compare both
feature configurations and preserve existing SPI/scheduler regressions.

`crates/core/tests/sam_dmac_byte.rs` declares90 authored guest cases per feature
configuration. Reset must interrupt actual partial progress, not just precede
all work. The second-beat read/write-error controls require63 beats remaining;
the failed beat must not decrement. A separate failed-writeback control cannot
produce TCMPL. Handler-written flag/vector/count witnesses and architectural
entry/return are checked, separately from host-MMIO/refusal/shared-vector tests.
The model is bus-assisted and not event-migrated or performance-qualified.

The reset guest disables the controller before SWRST. Enabled global reset
returns an access error, enabled channel reset is ignored, and LVLEN fields are
enable-protected. Separate host controls assert those boundaries and refusal
of excluded CRC control rather than silently accepting it.

Unsupported descriptor modes latch an explicit simulator refusal exposed by
subsequent register reads and inspection; no guessed hardware flags are used.
Writes that request unsupported trigger/command/priority/event modes refuse
immediately. External data/writeback side effects already performed before a
bus failure are not rolled back or claimed atomic. Wider transfers, physical
arbitration, CPU batch quanta and full production clock behavior are excluded.

Unsupported requests must fail explicitly, not silently degrade into byte
transfers or completion. The first slice excludes peripheral triggers, linked
descriptors, halfword/word beats, flexible steps, suspend/resume, CRC/events,
hardware arbitration/timing and production clock binding.

## Primary ambiguity retained

Section22.6.2.9 describes invalid VALID as SUSP/FERR; section22.8.22 also says
an invalid fetched descriptor sets TERR. Section22.10.1 describes suspension.
Do not invent a resolution or use invalid VALID as proof of one interrupt
classification. This first slice must refuse that case before a data beat or
success. Resolve the primary/errata boundary separately before advertising it.

The printed addressing formula uses BEATSIZE+1, while the field encoding and
width description need care for wider beats. The byte-only slice has an
unambiguous unit increment; wider beat/address math remains a separate review.

## Follow-on, not reached

Add live SERCOM DRE/RXC triggers, production AHB clock gating, descriptor
chaining/abort/suspend/error distinctions and final SPI TXC controls. Then
qualify actual CODAL callback/event/fiber behavior and the selected no-ASF build
closure. Only admitted execution can support active native/WASM Arcade RTx
and app/debugger adoption. A passing byte foundation does not complete P4.
