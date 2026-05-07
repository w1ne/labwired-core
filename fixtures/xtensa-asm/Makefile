TOOL = $(HOME)/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/xtensa-esp-elf/bin
AS       = $(TOOL)/xtensa-esp32s3-elf-as
LD       = $(TOOL)/xtensa-esp32s3-elf-ld
OBJCOPY  = $(TOOL)/xtensa-esp32s3-elf-objcopy
OBJDUMP  = $(TOOL)/xtensa-esp32s3-elf-objdump

I2C_FIXTURES = i2c0_empty_cmdlist i2c0_unmatched_addr_nack i2c0_fifo_pushes

all: fibonacci.elf fibonacci.bin $(I2C_FIXTURES:%=%.elf)

fibonacci.elf: fibonacci.s linker.ld
	$(AS) --no-transform $< -o fibonacci.o
	$(LD) -T linker.ld fibonacci.o -o $@

fibonacci.bin: fibonacci.elf
	$(OBJCOPY) -O binary $< $@

%.elf: %.s linker.ld
	$(AS) --no-transform $< -o $*.o
	$(LD) -T linker.ld $*.o -o $@

disasm: fibonacci.elf
	$(OBJDUMP) -d $<

clean:
	rm -f *.o *.elf *.bin

.PHONY: all disasm clean
