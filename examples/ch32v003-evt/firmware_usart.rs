#![no_std]
#![no_main]

use core::arch::global_asm;

// Reset-HSI USART1 transmit and PD0 LED loop on the CH32V003. This remains
// freestanding assembly so it does not rely on an RV32E Rust calling ABI.
global_asm!(
    r#"
    .section .text.entry
    .global _start
    .type _start,@function
_start:
    .option push
    .option rvc
    lui   t0, 0x40021          # RCC @ 0x40021000
    lui   t1, 0x4              # APB2PCENR: USART1EN (bit 14)
    addi  t1, t1, 0x20         # and GPIODEN (bit 5)
    sw    t1, 0x18(t0)

    lui   t0, 0x40011          # APB2 GPIO window @ 0x40011000
    addi  t0, t0, 0x400        # GPIOD @ 0x40011400
    lui   t1, 0xb00            # PD5 AF push-pull; PD0 output push-pull
    addi  t1, t1, 1
    sw    t1, 0(t0)             # CFGLR

    lui   t0, 0x40014          # USART1 base rounded up
    addi  t0, t0, -2048        # USART1 @ 0x40013800
    addi  t1, zero, 0xd0       # BRR = 24 MHz / 115200 (8N1)
    sw    t1, 0x08(t0)
    lui   t1, 0x2
    addi  t1, t1, 0x8          # CTLR1: UE | TE
    sw    t1, 0x0c(t0)

    la    a0, .Lready
1:
    lbu   t2, 0(a0)
    beqz  t2, 4f
2:
    lw    t1, 0(t0)            # ST: wait for TXE
    andi  t1, t1, 0x80
    beqz  t1, 2b
    sw    t2, 0x04(t0)         # DATAR
    addi  a0, a0, 1
    j     1b

4:
    lui   t0, 0x40011          # GPIOD BSHR
    addi  t0, t0, 0x410
5:
    addi  t2, zero, 1500
6:  addi  t2, t2, -1
    bnez  t2, 6b
    addi  t1, zero, 1
    sw    t1, 0(t0)            # set PD0
    addi  t2, zero, 1500
7:  addi  t2, t2, -1
    bnez  t2, 7b
    lui   t1, 0x10             # BSHR[31:16]: reset PD0
    sw    t1, 0(t0)
    j     5b
    .option pop

    .section .rodata
.Lready:
    .asciz "CH32V003_USART_OK\n"
    "#
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
