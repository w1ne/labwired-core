/*
 * Minimal GNU linker script for MKW41Z512xxx4 (KW41Z, Cortex-M0+).
 * Authored for this fixture (NOT the part under test). Provides the symbols
 * the vendor startup_MKW41Z4.S expects: __StackTop, __isr_vector, __etext,
 * __data_start__/__data_end__, __bss_start__/__bss_end__.
 *
 * Memory map:
 *   FLASH 512K @ 0x00000000 (vector table @ 0x0, flash config field @ 0x400)
 *   SRAM  128K @ 0x1FFF8000, stack top @ 0x20018000
 */

MEMORY
{
    m_interrupts (RX) : ORIGIN = 0x00000000, LENGTH = 0x00000200
    m_flash_config (RX) : ORIGIN = 0x00000400, LENGTH = 0x00000010
    m_text       (RX) : ORIGIN = 0x00000410, LENGTH = 0x0007FBF0
    m_data       (RW) : ORIGIN = 0x1FFF8000, LENGTH = 0x00020000
}

__StackTop = ORIGIN(m_data) + LENGTH(m_data); /* 0x20018000 */
__stack    = __StackTop;
_estack    = __StackTop;

ENTRY(Reset_Handler)

SECTIONS
{
    .interrupts :
    {
        __VECTOR_TABLE = .;
        __isr_vector = .;
        KEEP(*(.isr_vector))
        . = ALIGN(4);
    } > m_interrupts

    .flash_config :
    {
        . = ALIGN(4);
        KEEP(*(.FlashConfig))
        . = ALIGN(4);
    } > m_flash_config

    .text :
    {
        . = ALIGN(4);
        *(.text)
        *(.text*)
        *(.rodata)
        *(.rodata*)
        *(.glue_7)
        *(.glue_7t)
        *(.eh_frame)
        KEEP(*(.init))
        KEEP(*(.fini))
        . = ALIGN(4);
    } > m_text

    .ARM.extab :
    {
        *(.ARM.extab* .gnu.linkonce.armextab.*)
    } > m_text

    .ARM :
    {
        __exidx_start = .;
        *(.ARM.exidx*)
        __exidx_end = .;
    } > m_text

    .ctors :
    {
        __CTOR_LIST__ = .;
        KEEP (*crtbegin.o(.ctors))
        KEEP (*crtbegin?.o(.ctors))
        KEEP (*(EXCLUDE_FILE(*crtend?.o *crtend.o) .ctors))
        KEEP (*(SORT(.ctors.*)))
        KEEP (*(.ctors))
        __CTOR_END__ = .;
    } > m_text

    .dtors :
    {
        __DTOR_LIST__ = .;
        KEEP (*crtbegin.o(.dtors))
        KEEP (*crtbegin?.o(.dtors))
        KEEP (*(EXCLUDE_FILE(*crtend?.o *crtend.o) .dtors))
        KEEP (*(SORT(.dtors.*)))
        KEEP (*(.dtors))
        __DTOR_END__ = .;
    } > m_text

    .preinit_array :
    {
        PROVIDE_HIDDEN (__preinit_array_start = .);
        KEEP (*(.preinit_array*))
        PROVIDE_HIDDEN (__preinit_array_end = .);
    } > m_text

    .init_array :
    {
        PROVIDE_HIDDEN (__init_array_start = .);
        KEEP (*(SORT(.init_array.*)))
        KEEP (*(.init_array*))
        PROVIDE_HIDDEN (__init_array_end = .);
    } > m_text

    .fini_array :
    {
        PROVIDE_HIDDEN (__fini_array_start = .);
        KEEP (*(SORT(.fini_array.*)))
        KEEP (*(.fini_array*))
        PROVIDE_HIDDEN (__fini_array_end = .);
    } > m_text

    __etext = .; /* end of code, start of .data initializer image in flash */

    .data : AT(__etext)
    {
        . = ALIGN(4);
        __data_start__ = .;
        *(.data)
        *(.data*)
        . = ALIGN(4);
        __data_end__ = .;
    } > m_data

    .bss (NOLOAD) :
    {
        . = ALIGN(4);
        __bss_start__ = .;
        *(.bss)
        *(.bss*)
        *(COMMON)
        . = ALIGN(4);
        __bss_end__ = .;
    } > m_data

    .heap (NOLOAD) :
    {
        . = ALIGN(8);
        __end__ = .;
        PROVIDE(end = .);
        PROVIDE(_end = .);
    } > m_data

    .ARM.attributes 0 : { *(.ARM.attributes) }
}
