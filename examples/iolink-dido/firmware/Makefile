# IO-Link DI/DO device firmware — standard STM32CubeL4 project for the simulated
# STM32L476 (NUCLEO-L476RG). The real iolinki device stack is linked against the
# ST CMSIS device pack: official startup_stm32l476xx.s, system_stm32l4xx.c and
# the NUCLEO-L476RG linker script. Override STM32CUBE_L4_DIR if the pack lives
# elsewhere (default: a sibling checkout next to this repo).
CC      := arm-none-eabi-gcc
IOLINKI_DIR ?= ../../../third_party/iolinki
STM32CUBE_L4_DIR ?= ../../../../STM32CubeL4

CMSIS_DEV     := $(STM32CUBE_L4_DIR)/Drivers/CMSIS/Device/ST/STM32L4xx
CMSIS_CORE    := $(STM32CUBE_L4_DIR)/Drivers/CMSIS/Include
SYSTEM_SRC    := $(CMSIS_DEV)/Source/Templates/system_stm32l4xx.c
STARTUP_SRC   := $(CMSIS_DEV)/Source/Templates/gcc/startup_stm32l476xx.s
LINKER_SCRIPT := $(STM32CUBE_L4_DIR)/Projects/NUCLEO-L476RG/Templates/STM32CubeIDE/STM32L476RGTX_FLASH.ld

CFLAGS  := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -ffreestanding -O0 -g3 \
           -Wall -Wextra -ffunction-sections -fdata-sections -DSTM32L476xx \
           -I$(CMSIS_DEV)/Include -I$(CMSIS_CORE) \
           -I$(IOLINKI_DIR)/include -I.
LDFLAGS := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostartfiles \
           --specs=nano.specs --specs=nosys.specs -T $(LINKER_SCRIPT) -Wl,--gc-sections

# iolinki device-stack sources (the CMake IOLINKI_SOURCES list, minus the POSIX
# phy_virtual.c / linux platform, plus the baremetal time source).
IOLINKI_SRC := \
  $(IOLINKI_DIR)/src/iolink_core.c \
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

APP_SRC  := debug_uart.c phy_labwired.c main.c
CUBE_SRC := $(STARTUP_SRC) $(SYSTEM_SRC)
ELF      := iolink_dido.elf

all: $(ELF)

$(ELF): $(APP_SRC) $(IOLINKI_SRC) $(CUBE_SRC) $(LINKER_SCRIPT)
	$(CC) $(CFLAGS) $(LDFLAGS) $(APP_SRC) $(IOLINKI_SRC) $(CUBE_SRC) -o $@

clean:
	rm -f $(ELF)

.PHONY: all clean
