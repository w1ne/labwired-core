#![no_std]
#![no_main]

use core::arch::global_asm;

// Freestanding RV32EC transaction so this fixture does not rely on Rust's
// incomplete RV32E calling convention. It enables the CH32V003 GPIOC, GPIOD,
// USART1, and I2C1 gates; writes the SHT31 read-status command 0xF32D; reads the
// two-byte 0x8010 status value; and prints a pass marker only for that value.
global_asm!(
    r#"
    .section .text.entry
    .global _start
    .type _start,@function
_start:
    .option push
    .option rvc

    # RCC gates: GPIOC + GPIOD + USART1 (APB2PCENR), I2C1 (APB1PCENR).
    lui   t0, 0x40021
    lui   t1, 0x4
    addi  t1, t1, 0x30
    sw    t1, 0x18(t0)
    lui   t1, 0x200            # I2C1EN, APB1PCENR bit 21
    sw    t1, 0x1c(t0)

    # PC1/SDA and PC2/SCL as 50 MHz alternate-function open-drain.
    lui   t0, 0x40011
    lui   t1, 1
    addi  t1, t1, -16
    sw    t1, 0(t0)            # GPIOC CFGLR = 0x00000FF0

    # PD5 USART1_TX alternate-function push-pull, PD0 output left unused.
    addi  t0, t0, 0x400       # GPIOD
    lui   t1, 0xb00
    addi  t1, t1, 1
    sw    t1, 0(t0)

    # USART1: 24 MHz / 115200, 8N1, transmit only.
    lui   t0, 0x40014
    addi  t0, t0, -2048       # 0x40013800
    addi  t1, zero, 0xd0
    sw    t1, 0x08(t0)        # BRR
    lui   t1, 0x2
    addi  t1, t1, 0x8
    sw    t1, 0x0c(t0)        # CTLR1: UE | TE

    # CH32V003 I2C1 at 0x40005400, legacy CR1/CR2/DR event subset.
    lui   t0, 0x40005
    addi  t0, t0, 0x400
    addi  t1, zero, 24
    sw    t1, 0x04(t0)        # CTLR2.FREQ = 24 MHz
    lui   t1, 0x4
    sw    t1, 0x08(t0)        # OADDR1 mandatory bit 14
    addi  t1, zero, 120
    sw    t1, 0x1c(t0)        # CKCFGR: 100 kHz standard mode
    addi  t1, zero, 25
    sw    t1, 0x20(t0)        # RTR: 24 MHz + 1
    addi  t1, zero, 1
    sw    t1, 0(t0)           # CTLR1.PE

    # START + address 0x44/W, then issue SHT31 read-status command 0xF32D.
    addi  t1, zero, 0x101
    sw    t1, 0(t0)
1:  lw    t1, 0x14(t0)       # STAR1.SB
    andi  t1, t1, 1
    beqz  t1, 1b
    addi  t1, zero, 0x88
    sw    t1, 0x10(t0)        # DATAR = 0x44 << 1 | W
2:  lw    t1, 0x14(t0)       # STAR1.ADDR
    andi  t1, t1, 2
    beqz  t1, 2b
    lw    t1, 0x14(t0)       # clear ADDR: read STAR1 then STAR2
    lw    t1, 0x18(t0)
    addi  t1, zero, 0xf3
    sw    t1, 0x10(t0)
3:  lw    t1, 0x14(t0)       # wait for byte transfer finished
    andi  t1, t1, 4
    beqz  t1, 3b
    addi  t1, zero, 0x2d
    sw    t1, 0x10(t0)
4:  lw    t1, 0x14(t0)
    andi  t1, t1, 4
    beqz  t1, 4b
    addi  t1, zero, 0x201
    sw    t1, 0(t0)           # STOP completes the command write

    # New read transaction; POS+ACK prepares the standard two-byte receive.
    lui   t1, 1
    addi  t1, t1, -767      # 0xD01: PE | START | ACK | POS
    sw    t1, 0(t0)
    addi  t1, zero, 0x89
    sw    t1, 0x10(t0)        # DATAR = 0x44 << 1 | R
5:  lw    t1, 0x14(t0)       # STAR1.ADDR
    andi  t1, t1, 2
    beqz  t1, 5b
    lw    t1, 0x14(t0)
    lw    t1, 0x18(t0)
    lui   t1, 1
    addi  t1, t1, -2047      # 0x801: ACK=0, POS=1 for the final two bytes
    sw    t1, 0(t0)
6:  lw    t1, 0x14(t0)       # STAR1.BTF
    andi  t1, t1, 4
    beqz  t1, 6b
    addi  t1, zero, 0x201
    sw    t1, 0(t0)           # STOP before consuming the final pair
    lw    a0, 0x10(t0)        # status MSB
    lw    a1, 0x10(t0)        # status LSB

    addi  t1, zero, 0x80
    bne   a0, t1, .Lfail
    addi  t1, zero, 0x10
    bne   a1, t1, .Lfail

    la    a0, .Lok
    j     .Lprint
.Lfail:
    la    a0, .Lbad
.Lprint:
    lui   t0, 0x40014
    addi  t0, t0, -2048
.Lnext:
    lbu   t2, 0(a0)
    beqz  t2, .Ldone
.Ltxe:
    lw    t1, 0(t0)
    andi  t1, t1, 0x80
    beqz  t1, .Ltxe
    sw    t2, 0x04(t0)
    addi  a0, a0, 1
    j     .Lnext
.Ldone:
    j     .Ldone

    .section .rodata
.Lok:
    .asciz "CH32V003_SHT31_STATUS_OK\n"
.Lbad:
    .asciz "CH32V003_SHT31_STATUS_FAIL\n"
    .option pop
    "#
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
