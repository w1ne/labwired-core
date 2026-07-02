TARGET := j1939_monitor
BUILD_DIR := build
CROSS ?= arm-none-eabi-
CC := $(CROSS)gcc

CPUFLAGS := -mcpu=cortex-m3 -mthumb
CFLAGS := $(CPUFLAGS) -ffreestanding -fno-builtin -ffunction-sections \
          -fdata-sections -Os -g -Wall -Wextra -Werror
LDFLAGS := -nostdlib -T minimal.ld -Wl,--gc-sections -Wl,-Map=$(BUILD_DIR)/$(TARGET).map

SRCS := startup.c main.c
OBJS := $(patsubst %.c,$(BUILD_DIR)/%.o,$(SRCS))
ELF := $(BUILD_DIR)/$(TARGET).elf

all: $(ELF)

$(ELF): $(OBJS) minimal.ld
	$(CC) $(CPUFLAGS) $(LDFLAGS) $(OBJS) -o $@

$(BUILD_DIR)/%.o: %.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD_DIR):
	mkdir -p $@

clean:
	rm -rf $(BUILD_DIR)

.PHONY: all clean
