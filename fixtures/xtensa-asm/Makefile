TOOL = $(HOME)/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/xtensa-esp-elf/bin
AS       = $(TOOL)/xtensa-esp32s3-elf-as
LD       = $(TOOL)/xtensa-esp32s3-elf-ld
OBJCOPY  = $(TOOL)/xtensa-esp32s3-elf-objcopy
OBJDUMP  = $(TOOL)/xtensa-esp32s3-elf-objdump

all: fibonacci.elf fibonacci.bin

fibonacci.elf: fibonacci.s linker.ld
	$(AS) --no-transform $< -o fibonacci.o
	$(LD) -T linker.ld fibonacci.o -o $@

fibonacci.bin: fibonacci.elf
	$(OBJCOPY) -O binary $< $@

disasm: fibonacci.elf
	$(OBJDUMP) -d $<

clean:
	rm -f fibonacci.o fibonacci.elf fibonacci.bin

.PHONY: all disasm clean
