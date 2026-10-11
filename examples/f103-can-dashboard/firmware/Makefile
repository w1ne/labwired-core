# Three STM32F103 firmwares for the CAN dashboard demo.
#   make            engine.elf dashboard.elf sensor_500k.elf sensor_250k.elf
#   make install    copy the ELFs to ../prebuilt (the copies the tests load)
CROSS ?= arm-none-eabi-
CC := $(CROSS)gcc
BUILD := build

CPUFLAGS := -mcpu=cortex-m3 -mthumb
CFLAGS := $(CPUFLAGS) -std=c11 -ffreestanding -fno-builtin -ffunction-sections \
          -fdata-sections -fno-tree-loop-distribute-patterns -Os -g0 \
          -Wall -Wextra -Werror -Wno-override-init
LDFLAGS := -nostdlib -nostartfiles -T stm32f103c8.ld -Wl,--gc-sections

ELFS := engine dashboard sensor_500k sensor_250k
TARGETS := $(addprefix $(BUILD)/,$(addsuffix .elf,$(ELFS)))
HDRS := common.h font.h

all: $(TARGETS)

$(BUILD)/startup.o: startup.c | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/engine.elf: engine.c $(HDRS) $(BUILD)/startup.o stm32f103c8.ld
	$(CC) $(CFLAGS) $(LDFLAGS) engine.c $(BUILD)/startup.o -o $@

$(BUILD)/dashboard.elf: dashboard.c $(HDRS) $(BUILD)/startup.o stm32f103c8.ld
	$(CC) $(CFLAGS) $(LDFLAGS) dashboard.c $(BUILD)/startup.o -o $@

# The field bug is one number: 250000 instead of 500000.
$(BUILD)/sensor_500k.elf: sensor.c $(HDRS) $(BUILD)/startup.o stm32f103c8.ld
	$(CC) $(CFLAGS) -DCAN_BITRATE=500000u $(LDFLAGS) sensor.c $(BUILD)/startup.o -o $@

$(BUILD)/sensor_250k.elf: sensor.c $(HDRS) $(BUILD)/startup.o stm32f103c8.ld
	$(CC) $(CFLAGS) -DCAN_BITRATE=250000u $(LDFLAGS) sensor.c $(BUILD)/startup.o -o $@

$(BUILD):
	mkdir -p $@

install: all
	mkdir -p ../prebuilt
	cp $(TARGETS) ../prebuilt/

clean:
	rm -rf $(BUILD)

.PHONY: all install clean
