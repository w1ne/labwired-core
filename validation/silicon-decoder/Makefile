CLANG    ?= clang
RUST_LLD ?= $(shell find $(shell rustc --print sysroot) -name rust-lld | head -1)

CPUFLAGS := --target=arm-none-eabi -mcpu=cortex-m4 -mthumb
CFLAGS   := $(CPUFLAGS) -ffreestanding -fno-builtin -ffunction-sections \
            -fdata-sections -Os -g -Wall -Wextra -Werror

BUILD := build
ELF   := $(BUILD)/silicon_decoder.elf

all: $(ELF)

$(BUILD)/main.o: main.c | $(BUILD)
	$(CLANG) $(CFLAGS) -c $< -o $@

$(ELF): $(BUILD)/main.o minimal.ld
	$(RUST_LLD) -flavor gnu -m armelf -T minimal.ld --gc-sections \
	    -Map=$(BUILD)/silicon_decoder.map $(BUILD)/main.o -o $@

$(BUILD):
	mkdir -p $(BUILD)

clean:
	rm -rf $(BUILD)

.PHONY: all clean
