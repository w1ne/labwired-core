# IO-Link DI/DO IO-Link DI device firmware for the simulated STM32L476.
# M1: startup + debug UART only. iolinki sources + PHY are added in M2.
CC      := arm-none-eabi-gcc
IOLINKI_DIR ?= ../../../third_party/iolinki

CFLAGS  := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -ffreestanding -O0 -g3 \
           -Wall -Wextra -ffunction-sections -fdata-sections \
           -I$(IOLINKI_DIR)/include -I.
LDFLAGS := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostartfiles \
           --specs=nano.specs --specs=nosys.specs -T l476.ld -Wl,--gc-sections

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

APP_SRC := startup.c debug_uart.c phy_labwired.c main.c
ELF     := iolink_dido.elf

all: $(ELF)

$(ELF): $(APP_SRC) $(IOLINKI_SRC) l476.ld
	$(CC) $(CFLAGS) $(LDFLAGS) $(APP_SRC) $(IOLINKI_SRC) -o $@

clean:
	rm -f $(ELF)

.PHONY: all clean
