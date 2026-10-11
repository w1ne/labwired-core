# Native Cortex-A7 CPU fixture

See [the backend guide](../../docs/qemu-arm-core.md) for dependencies, exact commands,
fixture compiler/hash and validation receipts. `smoke.yaml` checks 12 in shared RAM
at `0x20000000` after ten ARM instructions. `firmware.S` and the checked-in ELF are
reproducible without Linux or vendor blobs.

This initialized-memory test platform is separate from the
[Luckfox Pico Mini A declaration](../luckfox-pico-mini-a/README.md). It contains
no RV1103 device models, boot chain, Linux image or physical validation.
