/*
 * Bare-metal PolarFire SoC MSS GPIO fixture for the imported polarfire-soc,
 * mpfs-icicle-kit and beaglev-fire profiles: one RV64IMAC U54 hart executes
 * from DDR at 0x8000_0000. It drives the three MSS GPIO banks through the
 * register map in Microchip's mss_gpio.h (GPIO_TypeDef) and prints what each
 * bank reads back on MMUART1 (16550, reg-shift 2):
 *
 *   GPIO_2 (32 ports): CFG_ALL = OUTPUT_MODE, GPIO_OUT = 0xA5A5, then
 *                      SET_BITS bit 16 and CLR_BITS bit 0 -> OUT 0x0001A5A4
 *   GPIO_0 (14 ports): CFG_ALL = OUTPUT_MODE | INPUT_MODE (0x7, buffer on),
 *                      GPIO_OUT = all ones -> the pads loop back: IN 0x3FFF
 *   GPIO_1 (24 ports): CFG_BYTE[1] = OUTPUT_MODE, GPIO_OUT = all ones
 *                      -> only ports 8..15 are outputs: OUT 0xFF00
 *
 * A zero-answering stub prints 00000000 for all three. No interrupt, PLIC or
 * other peripheral is exercised.
 */
.equ GPIO0, 0x20120000
.equ GPIO1, 0x20121000
.equ GPIO2, 0x20122000
.equ UART1, 0x20100000

.equ GPIO_IN, 0x84
.equ GPIO_OUT, 0x88
.equ GPIO_CFG_ALL, 0x8C
.equ GPIO_CFG_BYTE1, 0x94
.equ GPIO_CLR_BITS, 0xA0
.equ GPIO_SET_BITS, 0xA4

.equ UART_THR, 0x00
.equ UART_LSR, 0x14
.equ LSR_THRE, 1 << 5

.section .text.start, "ax", @progbits
.global _start
_start:
    li s0, GPIO2
    li t0, 0x5
    sw t0, GPIO_CFG_ALL(s0)
    li t0, 0xA5A5
    sw t0, GPIO_OUT(s0)
    li t0, 1 << 16
    sw t0, GPIO_SET_BITS(s0)
    li t0, 1
    sw t0, GPIO_CLR_BITS(s0)
    lw s3, GPIO_OUT(s0)

    li s1, GPIO0
    li t0, 0x7
    sw t0, GPIO_CFG_ALL(s1)
    li t0, -1
    sw t0, GPIO_OUT(s1)
    lw s4, GPIO_IN(s1)

    li s2, GPIO1
    li t0, 0x5
    sw t0, GPIO_CFG_BYTE1(s2)
    li t0, -1
    sw t0, GPIO_OUT(s2)
    lw s5, GPIO_OUT(s2)

    li s6, UART1
    la a0, banner
    jal ra, puts
    la a0, label2
    jal ra, puts
    mv a0, s3
    jal ra, puthex
    la a0, label0
    jal ra, puts
    mv a0, s4
    jal ra, puthex
    la a0, label1
    jal ra, puts
    mv a0, s5
    jal ra, puthex
    la a0, newline
    jal ra, puts
1:
    wfi
    j 1b

/* putc: t1 = byte. Clobbers t2. */
putc:
    lw t2, UART_LSR(s6)
    andi t2, t2, LSR_THRE
    beqz t2, putc
    sb t1, UART_THR(s6)
    ret

/* puts: a0 = NUL-terminated string. Clobbers a0, t1-t3. */
puts:
    mv t3, ra
2:
    lbu t1, 0(a0)
    beqz t1, 3f
    jal ra, putc
    addi a0, a0, 1
    j 2b
3:
    mv ra, t3
    ret

/* puthex: low 32 bits of a0 as eight hex digits. Clobbers a0-a2, t1-t3. */
puthex:
    mv t3, ra
    li a1, 28
4:
    srlw t1, a0, a1
    andi t1, t1, 0xF
    li a2, 10
    blt t1, a2, 5f
    addi t1, t1, 'a' - 10 - '0'
5:
    addi t1, t1, '0'
    jal ra, putc
    addi a1, a1, -4
    bgez a1, 4b
    mv ra, t3
    ret

.section .rodata
banner:  .asciz "MPFS MSS GPIO SMOKE\r\n"
label2:  .asciz "gpio2.out="
label0:  .asciz " gpio0.in="
label1:  .asciz " gpio1.out="
newline: .asciz "\r\n"
