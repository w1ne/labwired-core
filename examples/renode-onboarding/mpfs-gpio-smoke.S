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
 * A zero-answering stub prints 00000000 for all three.
 *
 * Then the PLIC (mss_plic.h PLIC_TypeDef, U54_1 = M-mode context 1) takes
 * GPIO_0's non-direct line, source 51 (PLIC_GPIO0_NON_DIRECT_INT_OFFSET):
 *
 *   SOURCE_PRIORITY[51] = 0xFF  -> reads 7 (three priority bits), then 2
 *   HART1_MMODE_ENA bit 51, threshold 0; claim with nothing pending -> 0
 *   GPIO_0 port 0 = OUT|IN|OUTBUFF|INT_ENABLE|edge-positive, OUT 0 -> 1
 *   -> MEIP traps with mcause 0x800000000000000b; the handler claims (51),
 *      clears GPIO_IRQ bit 0, completes; afterwards the pending word is 0.
 *   Restore CFG_ALL / GPIO_OUT with interrupts disabled; GPIO_IN is 0x3FFF
 *   again at the final memory assertion.
 *
 * Without a PLIC (or with the old zero-answering window) the trap never
 * comes and the line reads "plic ... id=00000000 ...".
 *
 * Last, MSS I2C0 (mss_i2c_regs.h; CoreI2C status codes) addresses 0x50,
 * which nothing on the imported board answers, with SI polled:
 *
 *   STATUS at reset                     -> 0xF8 (idle)
 *   CTRL = ENS1|STA                     -> SI, STATUS 0x08 (START sent),
 *                                          PLIC source 58 pending
 *   DATA = 0x50<<1 (SLA+W), clear SI    -> SI, STATUS 0x20 (address NACK)
 *   CTRL = ENS1|STO                     -> STATUS 0xF8, CTRL 0x40 (STO and
 *                                          SI clear)
 *
 * The zero-answering window this replaced reads STATUS 0x00, which on this
 * controller is the bus-error code, and never sets SI.
 *
 * Build (GNU binutils, riscv64-unknown-elf):
 *   as -march=rv64imac_zicsr -mabi=lp64 -o mpfs-gpio-smoke.o \
 *       examples/renode-onboarding/mpfs-gpio-smoke.S
 *   ld -N --no-warn-rwx-segments -Ttext=0x80000000 \
 *       -o tests/fixtures/mpfs-gpio-smoke.elf mpfs-gpio-smoke.o
 */
.equ GPIO0, 0x20120000
.equ GPIO1, 0x20121000
.equ GPIO2, 0x20122000
.equ UART1, 0x20100000
.equ PLIC, 0x0C000000
.equ I2C0, 0x2010A000

.equ I2C_CTRL, 0x00
.equ I2C_STATUS, 0x04
.equ I2C_DATA, 0x08
.equ I2C_SI, 1 << 3
.equ I2C_STO, 1 << 4
.equ I2C_STA, 1 << 5
.equ I2C_ENS1, 1 << 6
.equ PLIC_PRIO58, 4 * 58

.equ PLIC_PRIO51, 4 * 51
.equ PLIC_PENDING1, 0x1004
.equ PLIC_ENA_CTX1_W1, 0x2084
.equ PLIC_PAGE1, 0x1000
.equ PLIC_PAGE2, 0x2000
.equ PLIC_CTX1, 0x201000
.equ GPIO_IRQ, 0x80

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

    /* --- PLIC: GPIO_0 non-direct line, source 51 --- */
    la t0, trap
    csrw mtvec, t0
    li s7, 0            /* claimed ID, written by the handler */
    li s8, 0            /* mcause seen by the handler */
    li s9, PLIC
    li s10, PLIC + PLIC_CTX1
    li t0, 0xFF
    sw t0, PLIC_PRIO51(s9)
    lw s3, PLIC_PRIO51(s9)
    li t0, 2
    sw t0, PLIC_PRIO51(s9)
    li t0, 1 << (51 - 32)
    li t5, PLIC + PLIC_PAGE2
    sw t0, PLIC_ENA_CTX1_W1 - PLIC_PAGE2(t5)
    sw zero, 0(s10)     /* threshold */
    lw s4, 4(s10)       /* claim, nothing pending */
    li t0, 1 << 11
    csrs mie, t0        /* MEIE */
    csrsi mstatus, 1 << 3

    sw zero, GPIO_OUT(s1)
    li t0, 0x4F         /* OUT|IN|OUTBUFF|INT_ENABLE, INTTYPE 2 (rising) */
    sw t0, 0(s1)
    li t0, 1
    sw t0, GPIO_OUT(s1) /* rising edge on port 0 */
    li t0, 1000
6:
    bnez s7, 7f
    addi t0, t0, -1
    bnez t0, 6b
7:
    csrci mstatus, 1 << 3
    /* Restore GPIO_0's all-ports output state with edge IRQs disabled. */
    li t0, 0x7
    sw t0, GPIO_CFG_ALL(s1)
    li t0, -1
    sw t0, GPIO_OUT(s1)
    li t5, PLIC + PLIC_PAGE1
    lw s5, PLIC_PENDING1 - PLIC_PAGE1(t5)

    la a0, label_prio
    jal ra, puts
    mv a0, s3
    jal ra, puthex
    la a0, label_claim0
    jal ra, puts
    mv a0, s4
    jal ra, puthex
    la a0, label_id
    jal ra, puts
    mv a0, s7
    jal ra, puthex
    la a0, label_mcause
    jal ra, puts
    srli a0, s8, 32
    jal ra, puthex
    mv a0, s8
    jal ra, puthex
    la a0, label_pend
    jal ra, puts
    mv a0, s5
    jal ra, puthex
    la a0, newline
    jal ra, puts

    /* --- MSS I2C0: START, SLA+W to an absent 0x50, STOP --- */
    li s9, PLIC
    li t0, 1
    sw t0, PLIC_PRIO58(s9)
    li s11, I2C0
    lbu s3, I2C_STATUS(s11)
    li t0, I2C_ENS1 | I2C_STA
    sb t0, I2C_CTRL(s11)
    jal ra, i2c_wait_si
    lbu s4, I2C_STATUS(s11)
    li t5, PLIC + PLIC_PAGE1
    lw s5, PLIC_PENDING1 - PLIC_PAGE1(t5)
    srli s5, s5, 58 - 32
    andi s5, s5, 1
    li t0, 0x50 << 1
    sb t0, I2C_DATA(s11)
    li t0, I2C_ENS1     /* clears STA and SI: send the address */
    sb t0, I2C_CTRL(s11)
    jal ra, i2c_wait_si
    lbu s7, I2C_STATUS(s11)
    li t0, I2C_ENS1 | I2C_STO
    sb t0, I2C_CTRL(s11)
    lbu s8, I2C_STATUS(s11)
    lbu s10, I2C_CTRL(s11)

    la a0, label_i2c
    jal ra, puts
    mv a0, s3
    jal ra, puthex
    la a0, label_start
    jal ra, puts
    mv a0, s4
    jal ra, puthex
    la a0, label_irq
    jal ra, puts
    mv a0, s5
    jal ra, puthex
    la a0, label_sla
    jal ra, puts
    mv a0, s7
    jal ra, puthex
    la a0, label_stop
    jal ra, puts
    mv a0, s8
    jal ra, puthex
    la a0, label_ctrl
    jal ra, puts
    mv a0, s10
    jal ra, puthex
    la a0, newline
    jal ra, puts
1:
    wfi
    j 1b

/* i2c_wait_si: poll I2C0 CTRL.SI (s11) for at most 1000 reads. Clobbers t0-t1. */
i2c_wait_si:
    li t1, 1000
8:
    lbu t0, I2C_CTRL(s11)
    andi t0, t0, I2C_SI
    bnez t0, 9f
    addi t1, t1, -1
    bnez t1, 8b
9:
    ret

/* Machine trap handler: claim, clear the GPIO latch, complete. */
.balign 4
trap:
    csrr s8, mcause
    lw s7, 4(s10)
    li t4, 1
    sw t4, GPIO_IRQ(s1)
    sw s7, 4(s10)
    mret

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
label_prio:   .asciz "plic prio="
label_claim0: .asciz " claim0="
label_id:     .asciz " id="
label_mcause: .asciz " mcause="
label_pend:   .asciz " pending="
label_i2c:    .asciz "i2c0 reset="
label_start:  .asciz " start="
label_irq:    .asciz " irq58="
label_sla:    .asciz " sla_w="
label_stop:   .asciz " stop="
label_ctrl:   .asciz " ctrl="
