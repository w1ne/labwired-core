# ESP32-C3 Super Mini blinky firmware build.
#
# Compiles a minimal bare-metal GPIO8 blink for the C3 (riscv32imc) with the
# Espressif RISC-V GCC toolchain, producing a bare ELF the LabWired simulator
# boots directly (no boot ROM).
#
# Toolchain: riscv32-esp-elf-gcc (from PlatformIO or ESP-IDF). Override CC if it
# lives elsewhere, e.g.:
#   make CC=$$HOME/.platformio/packages/toolchain-riscv32-esp/bin/riscv32-esp-elf-gcc

ifneq ($(origin CC),command line)
DISCOVERED_CC := $(shell command -v riscv32-esp-elf-gcc 2>/dev/null)
ifeq ($(strip $(DISCOVERED_CC)),)
DISCOVERED_CC := $(firstword $(wildcard \
        $(HOME)/.platformio/packages/toolchain-riscv32-esp/bin/riscv32-esp-elf-gcc \
        $(HOME)/.platformio/tools/toolchain-riscv32-esp/bin/riscv32-esp-elf-gcc \
        $(HOME)/.espressif/tools/riscv32-esp-elf/*/riscv32-esp-elf/bin/riscv32-esp-elf-gcc))
endif
CC := $(DISCOVERED_CC)
endif

CFLAGS  := -march=rv32imc_zicsr_zifencei -mabi=ilp32 -ffreestanding -O2 -g \
           -Wall -Wextra -ffunction-sections -fdata-sections -I.
LDFLAGS := -march=rv32imc_zicsr_zifencei -mabi=ilp32 -nostartfiles \
           -Wl,--gc-sections -T c3.ld -lgcc

APP_SRC := startup.S c3_uart.c main.c

ELF := esp32c3_blinky.elf

all: $(ELF)

$(ELF): $(APP_SRC) c3.ld
	@if [ -z "$(strip $(CC))" ]; then \
	  echo "ERROR: riscv32-esp-elf-gcc not found. Install via PlatformIO or ESP-IDF,"; \
	  echo "       or pass CC=/path/to/riscv32-esp-elf-gcc."; exit 1; fi
	$(CC) $(CFLAGS) $(APP_SRC) $(LDFLAGS) -o $@

clean:
	rm -f $(ELF) *.o

.PHONY: all clean
