# Builds the three fidelity-benchmark variants from one source.
#   build/control.elf   correct firmware (positive control)
#   build/clockbug.elf  forgets RCC_APB2ENR.USART1EN
#   build/rambug.elf    stores past the end of the 20 KB SRAM
BUILD_DIR := build
CROSS ?= arm-none-eabi-
CC := $(CROSS)gcc

CPUFLAGS := -mcpu=cortex-m3 -mthumb
CFLAGS := $(CPUFLAGS) -ffreestanding -fno-builtin -ffunction-sections \
          -fdata-sections -Os -g -Wall -Wextra -Werror
LDFLAGS := -nostdlib -T bench.ld -Wl,--gc-sections

SRCS := startup.c main.c

ELVES := $(BUILD_DIR)/control.elf $(BUILD_DIR)/clockbug.elf \
         $(BUILD_DIR)/gpiobug.elf $(BUILD_DIR)/rambug.elf

all: $(ELVES)

$(BUILD_DIR):
	mkdir -p $@

$(BUILD_DIR)/control.elf: $(SRCS) bench.ld | $(BUILD_DIR)
	$(CC) $(CFLAGS) $(LDFLAGS) $(SRCS) -o $@

$(BUILD_DIR)/clockbug.elf: $(SRCS) bench.ld | $(BUILD_DIR)
	$(CC) $(CFLAGS) -DSKIP_UART_CLOCK $(LDFLAGS) $(SRCS) -o $@

$(BUILD_DIR)/gpiobug.elf: $(SRCS) bench.ld | $(BUILD_DIR)
	$(CC) $(CFLAGS) -DGPIO_CLOCK_BUG $(LDFLAGS) $(SRCS) -o $@

$(BUILD_DIR)/rambug.elf: $(SRCS) bench.ld | $(BUILD_DIR)
	$(CC) $(CFLAGS) -DRAM_OVERFLOW $(LDFLAGS) $(SRCS) -o $@

clean:
	rm -rf $(BUILD_DIR)

.PHONY: all clean
