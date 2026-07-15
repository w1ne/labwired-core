# ESP32-C3 thermal-fingerprint firmware build.
#
# Compiles the application + the REAL vendored Melexis MLX90640 driver for the
# C3 (riscv32imc) with the Espressif RISC-V GCC toolchain, producing a bare ELF
# the LabWired simulator boots directly (no boot ROM). The driver runs
# ON-TARGET; its I²C platform hooks (mlx90640_i2c_c3.c) drive the simulated C3
# I²C0 controller's command-list engine.
#
# Toolchain: riscv32-esp-elf-gcc (from PlatformIO or ESP-IDF). Override CC if it
# lives elsewhere, e.g.:
#   make CC=$$HOME/.platformio/packages/toolchain-riscv32-esp/bin/riscv32-esp-elf-gcc

# Auto-discover the Espressif riscv32 GCC. `make` predefines CC=cc, so detect
# whether CC was set on the command line (origin = "command line") and only
# auto-discover otherwise.
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

MLX_DIR ?= ../../../third_party/mlx90640-library
IOLINKI_DIR ?= ../../../third_party/iolinki

CFLAGS  := -march=rv32imc -mabi=ilp32 -ffreestanding -O2 -g \
           -Wall -Wextra -ffunction-sections -fdata-sections \
           -I$(MLX_DIR)/headers -I$(IOLINKI_DIR)/include -I.
LDFLAGS := -march=rv32imc -mabi=ilp32 -nostartfiles \
           -Wl,--gc-sections -T c3.ld -lm -lgcc

# iolinki IO-Link DEVICE-stack sources compiled ON-TARGET for rv32imc (the CMake
# IOLINKI_SOURCES list, minus the POSIX phy_virtual.c / linux platform, plus the
# baremetal time source) — the same set the iolink-dido ARM build uses. The
# reentrant stack's device-context entry point is src/device.c (iolink_core.c
# is gone from the pinned submodule).
IOLINKI_SRC := \
  $(IOLINKI_DIR)/src/device.c \
  $(IOLINKI_DIR)/src/phy_generic.c \
  $(IOLINKI_DIR)/src/crc.c \
  $(IOLINKI_DIR)/src/dll.c \
  $(IOLINKI_DIR)/src/isdu.c \
  $(IOLINKI_DIR)/src/events.c \
  $(IOLINKI_DIR)/src/platform.c \
  $(IOLINKI_DIR)/src/params.c \
  $(IOLINKI_DIR)/src/data_storage.c \
  $(IOLINKI_DIR)/src/device_info.c \
  $(IOLINKI_DIR)/src/platform_stubs.c \
  $(IOLINKI_DIR)/src/platform/baremetal/time_utils.c

APP_SRC := startup.S c3_uart.c mlx90640_i2c_c3.c fingerprint.c fast_math.c \
           phy_c3_iolink.c main.c
DRV_SRC := $(MLX_DIR)/functions/MLX90640_API.c
ELF     := thermal_fingerprint.elf

all: $(ELF)

$(ELF): $(APP_SRC) $(DRV_SRC) $(IOLINKI_SRC) c3.ld
	@if [ -z "$(strip $(CC))" ]; then \
	  echo "ERROR: riscv32-esp-elf-gcc not found. Install via PlatformIO or ESP-IDF,"; \
	  echo "       or pass CC=/path/to/riscv32-esp-elf-gcc."; exit 1; fi
	$(CC) $(CFLAGS) $(APP_SRC) $(DRV_SRC) $(IOLINKI_SRC) $(LDFLAGS) -o $@

clean:
	rm -f $(ELF) *.o

.PHONY: all clean
