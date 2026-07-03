# IO-Link MASTER firmware — standard STM32CubeL4 project for the simulated
# STM32L476 (NUCLEO-L476RG). Links the real iolinki-master stack + shared device
# frame/crc helpers against the ST CMSIS device pack: official startup_stm32l476xx.s,
# system_stm32l4xx.c and the NUCLEO-L476RG linker script. Override STM32CUBE_L4_DIR
# if the pack lives elsewhere (default: a sibling checkout next to this repo).
CC := arm-none-eabi-gcc
IOLINKI_MASTER_DIR ?= ../../../third_party/iolinki-master
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
           -I$(IOLINKI_MASTER_DIR)/include -I$(IOLINKI_DIR)/include -I.
LDFLAGS := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostartfiles \
           --specs=nano.specs --specs=nosys.specs -T $(LINKER_SCRIPT) -Wl,--gc-sections

MASTER_SRC := \
  $(IOLINKI_MASTER_DIR)/src/master_port.c \
  $(IOLINKI_MASTER_DIR)/src/master_controller.c \
  $(IOLINKI_MASTER_DIR)/src/master_isdu.c \
  $(IOLINKI_MASTER_DIR)/src/master_parameters.c \
  $(IOLINKI_MASTER_DIR)/src/master_sio.c \
  $(IOLINKI_DIR)/src/frame.c \
  $(IOLINKI_DIR)/src/crc.c

APP_SRC  := debug_uart.c phy_labwired.c main.c
CUBE_SRC := $(STARTUP_SRC) $(SYSTEM_SRC)
ELF      := master.elf

all: $(ELF)

$(ELF): $(APP_SRC) $(MASTER_SRC) $(CUBE_SRC) $(LINKER_SCRIPT)
	$(CC) $(CFLAGS) $(LDFLAGS) $(APP_SRC) $(MASTER_SRC) $(CUBE_SRC) -o $@

clean:
	rm -f $(ELF)

.PHONY: all clean
