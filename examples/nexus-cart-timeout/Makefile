CC = arm-none-eabi-gcc
# Built as a real Cortex-M3 ELF; build outputs are not committed.
CFLAGS = -mcpu=cortex-m3 -mthumb -Os -g -ffreestanding -fno-builtin -Wall -Wextra -Werror -ffunction-sections -fdata-sections
LDFLAGS = -nostdlib -Wl,--gc-sections -T stm32f103c8.ld
SRCS = src/startup.c src/cart.c
.PHONY: all clean check
all: build/cart.elf
build/cart.elf: $(SRCS) stm32f103c8.ld
	mkdir -p build
	$(CC) $(CFLAGS) $(LDFLAGS) $(SRCS) -o $@
clean:
	rm -rf build

check: all
	./tests/run_cases.sh
