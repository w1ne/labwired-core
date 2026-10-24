# Builds the brake-ECU demo for Cortex-M3 and RV32IMC with clang + lld.
# -O0 -g: coverage maps instructions back to source, so keep the optimizer
# from merging or predicating branches (Renode and gcov give the same advice).
CLANG ?= clang
CFLAGS = -g -O0 -ffreestanding -nostdlib -fno-builtin -Wall -Wextra
SRCS = src/brake_ecu.c src/startup.c

all: build/brake-ecu-cm3.elf build/brake-ecu-rv32.elf

build/brake-ecu-cm3.elf: $(SRCS) cm3.ld
	mkdir -p build
	$(CLANG) --target=thumbv7m-none-eabi -mcpu=cortex-m3 $(CFLAGS) -fuse-ld=lld -Wl,-n -Wl,-Tcm3.ld $(SRCS) -o $@

build/brake-ecu-rv32.elf: $(SRCS) rv32.ld
	mkdir -p build
	$(CLANG) --target=riscv32-unknown-elf -march=rv32imc -mabi=ilp32 $(CFLAGS) -fuse-ld=lld -Wl,-n -Wl,-Trv32.ld $(SRCS) -o $@

clean:
	rm -rf build

.PHONY: all clean

# Refresh the committed fixtures the tests and docs use.
fixtures: all
	cp build/brake-ecu-cm3.elf ../../tests/fixtures/coverage-brake-ecu-cm3.elf
	cp build/brake-ecu-rv32.elf ../../tests/fixtures/coverage-brake-ecu-rv32.elf

.PHONY: fixtures
