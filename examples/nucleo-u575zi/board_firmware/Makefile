# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

TARGET := u575_hal_smoke
BUILD_DIR := build

# Same convention as examples/nucleo-h563zi/board_firmware (STM32CubeH5):
# checkout STM32CubeU5 as a sibling of the repo root.
STM32CUBE_U5_DIR ?= $(abspath ../../../../STM32CubeU5)

HAL_DIR := $(STM32CUBE_U5_DIR)/Drivers/STM32U5xx_HAL_Driver
CMSIS_DEVICE := $(STM32CUBE_U5_DIR)/Drivers/CMSIS/Device/ST/STM32U5xx
CMSIS_INCLUDE := $(STM32CUBE_U5_DIR)/Drivers/CMSIS/Include
STARTUP_SRC := $(CMSIS_DEVICE)/Source/Templates/gcc/startup_stm32u575xx.s
SYSTEM_SRC := $(CMSIS_DEVICE)/Source/Templates/system_stm32u5xx.c
LINKER_SCRIPT := $(CMSIS_DEVICE)/Source/Templates/gcc/linker/STM32U575xx_FLASH.ld

CC := arm-none-eabi-gcc
OBJCOPY := arm-none-eabi-objcopy
SIZE := arm-none-eabi-size

HAL_SRCS := \
	$(HAL_DIR)/Src/stm32u5xx_hal.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_cortex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_rcc.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_rcc_ex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_pwr.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_pwr_ex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_gpio.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_uart.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_flash.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_flash_ex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_icache.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_dma.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_dma_ex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_exti.c

CFLAGS := \
	-mcpu=cortex-m33 \
	-mthumb \
	-mfloat-abi=hard \
	-mfpu=fpv5-sp-d16 \
	-ffreestanding \
	-fno-builtin \
	-ffunction-sections \
	-fdata-sections \
	-fno-common \
	-Wall \
	-Wextra \
	-Os \
	-g3 \
	-DUSE_HAL_DRIVER \
	-DSTM32U575xx \
	-I. \
	-I$(CMSIS_DEVICE)/Include \
	-I$(CMSIS_INCLUDE) \
	-I$(HAL_DIR)/Inc

LDFLAGS := \
	-mcpu=cortex-m33 \
	-mthumb \
	-mfloat-abi=hard \
	-mfpu=fpv5-sp-d16 \
	-nostdlib \
	-Wl,--gc-sections \
	-Wl,-Map,$(BUILD_DIR)/$(TARGET).map \
	-T$(LINKER_SCRIPT)

MAIN_OBJ := $(BUILD_DIR)/main.o
SYSTEM_OBJ := $(BUILD_DIR)/system_stm32u5xx.o
STARTUP_OBJ := $(BUILD_DIR)/startup_stm32u575xx.o
HAL_OBJS := $(patsubst $(HAL_DIR)/Src/%.c,$(BUILD_DIR)/hal_%.o,$(HAL_SRCS))

ELF := $(BUILD_DIR)/$(TARGET).elf

.PHONY: all clean

all: $(ELF)
	$(SIZE) $(ELF)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(MAIN_OBJ): main.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(SYSTEM_OBJ): $(SYSTEM_SRC) | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(STARTUP_OBJ): $(STARTUP_SRC) | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD_DIR)/hal_%.o: $(HAL_DIR)/Src/%.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(ELF): $(MAIN_OBJ) $(SYSTEM_OBJ) $(STARTUP_OBJ) $(HAL_OBJS)
	$(CC) $(LDFLAGS) $^ -o $@

clean:
	rm -rf $(BUILD_DIR)
