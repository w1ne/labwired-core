# KiCad Drop-In CI Sample

Imports a real 11-part KiCad `.kicad_pcb` board (`handheld-carrier.kicad_pcb`, STM32F411 BlackPill carrier) **without hand-conversion** through `uses: w1ne/labwired-core@v1`:

1. Imports the KiCad board natively and maps footprints to LabWired catalog parts.
2. Runs Electrical Rule Check (ERC: 0 errors / 0 warnings) and copper PCB Design Rule Check (DRC: 0 errors / 0 warnings).
3. Lowers the imported KiCad board to a runnable `system.yaml` + `chip.yaml`.
4. Boots the STM32F411 firmware ELF and verifies deterministic UART assertions.
