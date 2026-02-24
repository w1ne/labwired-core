# LabWired - HIL Displacement Showcase Makefile
TARGET := hil_displacement_showcase
BUILD_DIR := build

STM32CUBE_H5_DIR ?= $(abspath ../../../../STM32CubeH5)

DEVICE_INCLUDE := $(STM32CUBE_H5_DIR)/Drivers/CMSIS/Device/ST/STM32H5xx/Include
CMSIS_INCLUDE := $(STM32CUBE_H5_DIR)/Drivers/CMSIS/Include
SYSTEM_SRC := $(STM32CUBE_H5_DIR)/Drivers/CMSIS/Device/ST/STM32H5xx/Source/Templates/system_stm32h5xx.c
STARTUP_SRC := $(STM32CUBE_H5_DIR)/Drivers/CMSIS/Device/ST/STM32H5xx/Source/Templates/gcc/startup_stm32h563xx.s
LINKER_SCRIPT := $(STM32CUBE_H5_DIR)/Drivers/CMSIS/Device/ST/STM32H5xx/Source/Templates/gcc/linker/STM32H563xx_FLASH.ld

CC := arm-none-eabi-gcc
OBJCOPY := arm-none-eabi-objcopy
SIZE := arm-none-eabi-size

CFLAGS := \
	-mcpu=cortex-m33 \
	-mthumb \
	-ffreestanding \
	-fno-builtin \
	-ffunction-sections \
	-fdata-sections \
	-fno-common \
	-Wall \
	-Wextra \
	-Os \
	-g3 \
	-DSTM32H563xx \
	-I$(DEVICE_INCLUDE) \
	-I$(CMSIS_INCLUDE)

LDFLAGS := \
	-mcpu=cortex-m33 \
	-mthumb \
	-nostdlib \
	-Wl,--gc-sections \
	-Wl,-Map,$(BUILD_DIR)/$(TARGET).map \
	-T$(LINKER_SCRIPT)

MAIN_OBJ := $(BUILD_DIR)/main.o
SYSTEM_OBJ := $(BUILD_DIR)/system_stm32h5xx.o
STARTUP_OBJ := $(BUILD_DIR)/startup_stm32h563xx.o

ELF := $(BUILD_DIR)/$(TARGET).elf
BIN := $(BUILD_DIR)/$(TARGET).bin

.PHONY: all clean

all: $(ELF) $(BIN)
	$(SIZE) $(ELF)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(MAIN_OBJ): main.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(SYSTEM_OBJ): $(SYSTEM_SRC) | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(STARTUP_OBJ): $(STARTUP_SRC) | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(ELF): $(MAIN_OBJ) $(SYSTEM_OBJ) $(STARTUP_OBJ)
	$(CC) $(LDFLAGS) $^ -o $@

$(BIN): $(ELF)
	$(OBJCOPY) -O binary $< $@

clean:
	rm -rf $(BUILD_DIR)
