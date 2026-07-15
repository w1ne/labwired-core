# ESP32-C3 Leo air-quality firmware build.
#
# Compiles the application + the REAL vendored Sensirion embedded-i2c drivers
# (SCD4x, SGP41, SPS30) and the Sensirion Gas Index Algorithm for the C3
# (riscv32imc) with the Espressif RISC-V GCC toolchain, producing a bare ELF the
# LabWired simulator boots directly (no boot ROM). The drivers run ON-TARGET;
# their I²C platform hooks (sensirion_i2c_hal_c3.c) drive the simulated C3 I²C0
# controller's command-list engine.
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

TP       ?= ../../../third_party
SCD_DIR  := $(TP)/embedded-i2c-scd4x
SGP_DIR  := $(TP)/embedded-i2c-sgp41
SPS_DIR  := $(TP)/embedded-i2c-sps30
GAS_DIR  := $(TP)/gas-index-algorithm/sensirion_gas_index_algorithm

CFLAGS  := -march=rv32imc -mabi=ilp32 -ffreestanding -O2 -g \
           -Wall -Wextra -ffunction-sections -fdata-sections \
           -I. -I$(SCD_DIR) -I$(SGP_DIR) -I$(SPS_DIR) -I$(GAS_DIR)
LDFLAGS := -march=rv32imc -mabi=ilp32 -nostartfiles \
           -Wl,--gc-sections -T c3.ld -lm -lgcc

# One copy of the shared Sensirion transport layer (identical across the three
# vendor repos) compiled from the SCD4x tree to avoid duplicate symbols.
SENSIRION_SHARED := \
  $(SCD_DIR)/sensirion_common.c \
  $(SCD_DIR)/sensirion_i2c.c

# Per-part vendor drivers (unmodified) + the Gas Index Algorithm.
SENSIRION_PARTS := \
  $(SCD_DIR)/scd4x_i2c.c \
  $(SGP_DIR)/sgp41_i2c.c \
  $(SPS_DIR)/sps30_i2c.c \
  $(GAS_DIR)/sensirion_gas_index_algorithm.c

# Application + platform shims (our code).
APP_SRC := startup.S c3_uart.c sensirion_i2c_hal_c3.c veml7700_c3.c mlx90614_c3.c ssd1306_c3.c main.c

ELF := leo_airquality.elf

all: $(ELF)

$(ELF): $(APP_SRC) $(SENSIRION_SHARED) $(SENSIRION_PARTS) c3.ld
	@if [ -z "$(strip $(CC))" ]; then \
	  echo "ERROR: riscv32-esp-elf-gcc not found. Install via PlatformIO or ESP-IDF,"; \
	  echo "       or pass CC=/path/to/riscv32-esp-elf-gcc."; exit 1; fi
	$(CC) $(CFLAGS) $(APP_SRC) $(SENSIRION_SHARED) $(SENSIRION_PARTS) $(LDFLAGS) -o $@

clean:
	rm -f $(ELF) *.o

.PHONY: all clean
