TARGET := obd2_ecu
BUILD_DIR := build
CROSS ?= arm-none-eabi-
CC := $(CROSS)gcc
HOST_CC ?= cc
CFLAGS := -mcpu=cortex-m3 -mthumb -std=c11 -ffreestanding -fno-builtin \
          -ffunction-sections -fdata-sections -Os -g -Wall -Wextra -Werror
LDFLAGS := -mcpu=cortex-m3 -mthumb -nostdlib -T minimal.ld -Wl,--gc-sections \
           -Wl,-Map=$(BUILD_DIR)/$(TARGET).map
OBJS := $(BUILD_DIR)/startup.o $(BUILD_DIR)/main.o $(BUILD_DIR)/obd2.o

all: $(BUILD_DIR)/$(TARGET).elf

host-test: $(BUILD_DIR)/obd2_test
	$(BUILD_DIR)/obd2_test

$(BUILD_DIR)/obd2_test: obd2_test.c obd2.c obd2.h | $(BUILD_DIR)
	$(HOST_CC) -std=c11 -Wall -Wextra -Werror obd2_test.c obd2.c -o $@

$(BUILD_DIR)/$(TARGET).elf: $(OBJS) minimal.ld
	$(CC) $(LDFLAGS) $(OBJS) -o $@

$(BUILD_DIR)/%.o: %.c obd2.h | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD_DIR):
	mkdir -p $@

clean:
	rm -rf $(BUILD_DIR)

.PHONY: all host-test clean
