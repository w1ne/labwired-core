/* R5F52618BGFP: 128 KiB RAM at 0, 512 KiB code flash at 0xFFF80000 whose
 * last word is the reset vector. */
MEMORY
{
	RAM (rwx) : ORIGIN = 0x00000000, LENGTH = 128K
	ROM (rx)  : ORIGIN = 0xFFF80000, LENGTH = 512K - 4
	RESET (r) : ORIGIN = 0xFFFFFFFC, LENGTH = 4
}
ENTRY(_reset)
SECTIONS
{
	.text : { *(.text.start) *(.text*) *(P*) *(.rodata*) *(C) *(C_1) *(C_2) *(W*) *(L) } > ROM
	.fvectors : { KEEP(*(.fvectors)) } > RESET
	.data : { *(.data*) *(.bss*) *(D*) *(B*) *(R*) *(COMMON) } > RAM
	__stack_top = ORIGIN(RAM) + LENGTH(RAM);
	/DISCARD/ : { *(.comment) }
}
