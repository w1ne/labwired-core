# AL2205 IO-Link DI device firmware for the simulated STM32L476.
# M1: startup + debug UART only. iolinki sources + PHY are added in M2.
CC      := arm-none-eabi-gcc
IOLINKI_DIR ?= ../../../third_party/iolinki

CFLAGS  := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -ffreestanding -Os -g3 \
           -Wall -Wextra -ffunction-sections -fdata-sections \
           -I$(IOLINKI_DIR)/include -I.
LDFLAGS := -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostartfiles \
           --specs=nano.specs -T l476.ld -Wl,--gc-sections

APP_SRC := startup.c debug_uart.c main.c
ELF     := al2205_dido.elf

all: $(ELF)

$(ELF): $(APP_SRC) l476.ld
	$(CC) $(CFLAGS) $(LDFLAGS) $(APP_SRC) -o $@

clean:
	rm -f $(ELF)

.PHONY: all clean
