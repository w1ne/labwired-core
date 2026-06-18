# IO-Link MASTER firmware for the simulated STM32L476.
# Builds the real iolinki-master stack + shared device frame/crc helpers.
CC := arm-none-eabi-gcc
IOLINKI_MASTER_DIR ?= ../../../third_party/iolinki-master
IOLINKI_DIR ?= ../../../third_party/iolinki

CFLAGS  := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -ffreestanding -O0 -g3 \
           -Wall -Wextra -ffunction-sections -fdata-sections \
           -I$(IOLINKI_MASTER_DIR)/include -I$(IOLINKI_DIR)/include -I.
LDFLAGS := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostartfiles \
           --specs=nano.specs --specs=nosys.specs -T l476.ld -Wl,--gc-sections

MASTER_SRC := \
  $(IOLINKI_MASTER_DIR)/src/master_port.c \
  $(IOLINKI_MASTER_DIR)/src/master_controller.c \
  $(IOLINKI_MASTER_DIR)/src/master_isdu.c \
  $(IOLINKI_MASTER_DIR)/src/master_parameters.c \
  $(IOLINKI_MASTER_DIR)/src/master_sio.c \
  $(IOLINKI_DIR)/src/frame.c \
  $(IOLINKI_DIR)/src/crc.c

APP_SRC := startup.c debug_uart.c phy_labwired.c main.c
ELF     := master.elf

all: $(ELF)

$(ELF): $(APP_SRC) $(MASTER_SRC) l476.ld
	$(CC) $(CFLAGS) $(LDFLAGS) $(APP_SRC) $(MASTER_SRC) -o $@

clean:
	rm -f $(ELF)

.PHONY: all clean
