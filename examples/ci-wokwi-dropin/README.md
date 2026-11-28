# Wokwi Drop-In CI Sample

Runs an **unmodified** Wokwi `diagram.json` (`board-stm32-blackpill-stm32f411` + `wokwi-ssd1306` + 4.7 kΩ I2C pull-up resistors) through `uses: w1ne/labwired-core@v1`:

1. Auto-detects Wokwi format and translates parts + pins to a canonical LabWired digital twin.
2. Runs Electrical Rule Check (ERC: 0 errors / 0 warnings).
3. Lowers to a runnable `system.yaml` + `chip.yaml` automatically.
4. Boots the STM32F411 firmware ELF and verifies deterministic UART assertions.
