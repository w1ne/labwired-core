#![no_std]
#![no_main]

use core::arch::global_asm;

// CH32V003 48 MHz bring-up and SysTick delays, as the WCH SDK and
// ch32v003fun perform them. The register sequences follow
// openwch/ch32v003 system_ch32v00x.c (SystemInit, SetSysClockTo_48MHZ_HSI)
// and debug.c (Delay_Us), and ch32v003fun's DelaySysTick. This remains
// freestanding assembly so it does not rely on an RV32E Rust calling ABI.
//
// Each delay counts its poll-loop iterations. A delay that returns at once
// (a counter already past CMP) or never advances (no SysTick) fails the
// check; only a bounded wait prints the OK marker.
global_asm!(
    r#"
    .section .text.entry
    .global _start
    .type _start,@function
_start:
    .option push
    .option rvc
    lui   s0, 0x40021          # RCC @ 0x40021000

    # --- SystemInit ---
    lw    t1, 0(s0)            # CTLR |= HSION
    ori   t1, t1, 1
    sw    t1, 0(s0)
    lw    t1, 4(s0)            # CFGR0 &= 0xFCFF0000
    li    t2, 0xFCFF0000
    and   t1, t1, t2
    sw    t1, 4(s0)
    lw    t1, 0(s0)            # CTLR &= 0xFEF6FFFF (PLLON, CSSON, HSEON)
    li    t2, 0xFEF6FFFF
    and   t1, t1, t2
    sw    t1, 0(s0)
    lw    t1, 0(s0)            # CTLR &= 0xFFFBFFFF (HSEBYP)
    li    t2, 0xFFFBFFFF
    and   t1, t1, t2
    sw    t1, 0(s0)
    lw    t1, 4(s0)            # CFGR0 &= 0xFFFEFFFF (PLLSRC = HSI)
    li    t2, 0xFFFEFFFF
    and   t1, t1, t2
    sw    t1, 4(s0)
    li    t1, 0x009F0000       # INTR: clear ready flags
    sw    t1, 8(s0)

    # --- SetSysClockTo_48MHZ_HSI ---
    lui   t0, 0x40022          # FLASH_ACTLR: LATENCY = 1
    lw    t1, 0(t0)
    andi  t1, t1, -4
    ori   t1, t1, 1
    sw    t1, 0(t0)
    lw    t1, 0(s0)            # CTLR |= PLLON
    li    t2, 0x01000000
    or    t1, t1, t2
    sw    t1, 0(s0)
    li    t2, 0x02000000
1:  lw    t1, 0(s0)            # wait PLLRDY
    and   t1, t1, t2
    beqz  t1, 1b
    lw    t1, 4(s0)            # SW = PLL
    andi  t1, t1, -4
    ori   t1, t1, 2
    sw    t1, 4(s0)
2:  lw    t1, 4(s0)            # wait SWS == PLL
    andi  t1, t1, 0xC
    addi  t1, t1, -8
    bnez  t1, 2b

    # GPIOD and USART1 clocks, PD0 push-pull out, PD5 USART1_TX AF push-pull.
    li    t1, 0x4020
    sw    t1, 0x18(s0)
    lui   t0, 0x40011
    addi  t0, t0, 0x400        # GPIOD @ 0x40011400
    lui   t1, 0xb00
    addi  t1, t1, 1
    sw    t1, 0(t0)            # CFGLR
    lui   s1, 0x40014
    addi  s1, s1, -2048        # USART1 @ 0x40013800
    li    t1, 0x1A1            # BRR = 48 MHz / 115200
    sw    t1, 0x08(s1)
    li    t1, 0x2008           # CTLR1: UE | TE
    sw    t1, 0x0c(s1)

    la    a0, .Lpll
    jal   puts

    lui   a4, 0xE000F          # STK @ 0xE000F000

    # --- WCH SDK Delay_Us(100): p_us = 48 MHz / 8 MHz = 6 -> CMP = 600 ---
    li    a2, 600
    jal   sdk_delay
    la    a0, .Lsdk_ok
    jal   check_bounds

    # --- ch32v003fun DelaySysTick(4800) with CTLR = 5 (STE | HCLK) ---
    li    t1, 5
    sw    t1, 0(a4)
    lw    t2, 8(a4)
    li    t1, 4800
    add   a5, t2, t1           # targend = CNT + n
    li    a3, 0
3:  addi  a3, a3, 1
    lw    t1, 8(a4)
    sub   t1, t1, a5
    bltz  t1, 3b               # while ((int32_t)(CNT - targend) < 0)
    sw    zero, 0(a4)          # back to the SDK's stopped state
    la    a0, .Lfun_ok
    jal   check_bounds

    # --- PD0 toggles every Delay_Us(100) ---
    lui   s0, 0x40011
    addi  s0, s0, 0x410        # GPIOD BSHR
4:  li    a2, 600
    jal   sdk_delay
    li    t1, 1
    sw    t1, 0(s0)            # set PD0
    li    a2, 600
    jal   sdk_delay
    lui   t1, 0x10             # BSHR[31:16]: reset PD0
    sw    t1, 0(s0)
    j     4b

# Delay_Us/Delay_Ms body: a2 = CMP; returns a3 = poll iterations.
sdk_delay:
    lw    t1, 4(a4)            # SR &= ~CNTIF
    andi  t1, t1, -2
    sw    t1, 4(a4)
    sw    a2, 0x10(a4)         # CMP
    sw    zero, 8(a4)          # CNT = 0
    lw    t1, 0(a4)            # CTLR |= STE
    ori   t1, t1, 1
    sw    t1, 0(a4)
    li    a3, 0
5:  addi  a3, a3, 1
    lw    t1, 4(a4)            # wait CNTIF
    andi  t1, t1, 1
    beqz  t1, 5b
    lw    t1, 0(a4)            # CTLR &= ~STE
    andi  t1, t1, -2
    sw    t1, 0(a4)
    ret

# 4800 core cycles of polling at four instructions per iteration is about
# 1200 iterations. Accept 300..=4800 (at least one cycle per iteration and at
# most sixteen); otherwise print the BAD marker. a0 = OK message.
check_bounds:
    mv    s0, ra
    li    t1, 300
    blt   a3, t1, 6f
    li    t1, 4800
    blt   t1, a3, 6f
    jal   puts
    mv    ra, s0
    ret
6:  la    a0, .Lbad
    jal   puts
    mv    ra, s0
    ret

# a0 = NUL-terminated string; s1 = USART1.
puts:
7:  lbu   t2, 0(a0)
    beqz  t2, 9f
8:  lw    t1, 0(s1)            # STATR: wait TXE
    andi  t1, t1, 0x80
    beqz  t1, 8b
    sw    t2, 0x04(s1)         # DATAR
    addi  a0, a0, 1
    j     7b
9:  ret
    .option pop

    .section .rodata
.Lpll:
    .asciz "CH32V003_PLL48_OK\n"
.Lsdk_ok:
    .asciz "CH32V003_SDK_DELAY_OK\n"
.Lfun_ok:
    .asciz "CH32V003_CH32FUN_DELAY_OK\n"
.Lbad:
    .asciz "CH32V003_SYSTICK_BAD\n"
"#
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
