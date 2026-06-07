/* Minimal Cortex-M4 startup for STM32L476 — no libc */

/* Symbols from linker script */
extern unsigned long _sidata; /* LMA of .data in flash */
extern unsigned long _sdata;  /* VMA start of .data in RAM */
extern unsigned long _edata;  /* VMA end of .data in RAM */
extern unsigned long _sbss;   /* start of .bss */
extern unsigned long _ebss;   /* end of .bss */
extern unsigned long _estack; /* top of stack (RAM end) */

/* User entry point */
extern int main(void);

/* Forward declaration */
void Reset_Handler(void) __attribute__((noreturn));

void Reset_Handler(void) {
    /* Copy .data from flash to RAM */
    unsigned long *src = &_sidata;
    unsigned long *dst = &_sdata;
    while (dst < &_edata) {
        *dst++ = *src++;
    }

    /* Zero .bss */
    dst = &_sbss;
    while (dst < &_ebss) {
        *dst++ = 0;
    }

    /* Call user main; loop if it returns */
    main();
    for (;;) {}
}

/* Minimal vector table: initial SP + Reset_Handler */
__attribute__((section(".vectors")))
const void *_vector_table[] = {
    (void *)&_estack,       /* Initial stack pointer */
    (void *)Reset_Handler,  /* Reset handler */
};
