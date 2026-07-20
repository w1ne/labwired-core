# Build the nRF54L15 onboarding smoke firmware.
#
# Requires arm-none-eabi-gcc on PATH. No Zephyr, no nRF Connect SDK, no CMSIS —
# the firmware is standalone C so that a boot failure localises to the chip
# profile rather than to a vendor HAL.
#
#   make            build ELF
#   make publish    build and copy to tests/fixtures/ (the committed fixture)
#   make clean

CROSS   ?= arm-none-eabi-
CC      := $(CROSS)gcc
OBJCOPY := $(CROSS)objcopy
SIZE    := $(CROSS)size

TARGET  := nrf54l15-smoke
BUILD   := build
FIXTURE := ../../tests/fixtures/$(TARGET).elf

SRCS := src/startup.c src/main.c
OBJS := $(patsubst src/%.c,$(BUILD)/%.o,$(SRCS))

# Cortex-M33 without FPU/DSP: the smoke path is integer-only, and building
# soft-float keeps the ELF free of FPU state the profile need not model yet.
ARCHFLAGS := -mcpu=cortex-m33 -mthumb -mfloat-abi=soft

CFLAGS  := $(ARCHFLAGS) -std=c11 -Os -g3 \
           -Wall -Wextra -Wpedantic -Werror \
           -ffreestanding -fno-common \
           -ffunction-sections -fdata-sections

LDFLAGS := $(ARCHFLAGS) -T nrf54l15.ld -nostdlib -nostartfiles \
           -Wl,--gc-sections -Wl,-Map=$(BUILD)/$(TARGET).map

all: $(BUILD)/$(TARGET).elf

$(BUILD):
	@mkdir -p $(BUILD)

$(BUILD)/%.o: src/%.c | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/$(TARGET).elf: $(OBJS)
	$(CC) $(OBJS) $(LDFLAGS) -o $@
	$(SIZE) $@

publish: $(BUILD)/$(TARGET).elf
	cp $< $(FIXTURE)
	@echo "Published $(FIXTURE)"

clean:
	rm -rf $(BUILD)

.PHONY: all publish clean
