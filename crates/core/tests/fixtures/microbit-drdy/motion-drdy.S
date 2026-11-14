/* SPDX-License-Identifier: MIT
 * Original TWIM0 EasyDMA driver and falling-edge GPIOTE observer.
 * RAM +00 line, +04 loops, +08 falls, +0c bus errors, +10 host command,
 * +14 completed command, +18 last read byte, +1c identity.
 * Commands change only firmware intent; the host never writes sensor/GPIO.
 */
.file "motion-drdy.S"
.syntax unified
.cpu cortex-m4
.thumb
.section .vectors,"a"
.word 0x20020000
.word reset
.section .text,"ax"
.thumb_func
.global reset
reset:
    ldr r0, =0x50000764    /* GPIO0.PIN_CNF[25] */
    movs r1, #0x0c        /* input connected, internal pull-up enabled */
    str r1, [r0]
    ldr r8, =0x40003000
    ldr r9, =0x20000200
    ldr r10, =0x20000100
    movs r0, #8
    str r0, [r8, #0x508]
    movs r0, #16
    str r0, [r8, #0x50c]
    ldr r0, =0x06400000
    str r0, [r8, #0x524]
    movs r0, #6
    str r0, [r8, #0x500]
    ldr r0, =0x40006510
    ldr r1, =0x00021901 /* P0.25 event, high to low */
    str r1, [r0]
    movs r0, #0x1e
    movs r1, #0x4f
    movs r2, #1
    add r3, r10, #0x1c
    bl motion_read
    movs r1, #0x60
    movs r2, #0x8c /* continuous 100 Hz, route remains reset off */
    bl motion_write
loop:
    ldr r0, =0x50000510
    ldr r1, [r0]
    lsrs r1, #25
    and r1, r1, #1
    str r1, [r10]
    ldr r1, [r10, #4]
    adds r1, #1
    str r1, [r10, #4]
    ldr r0, =0x40006100
    ldr r1, [r0]
    cbz r1, command
    movs r1, #0
    str r1, [r0]
    ldr r1, [r10, #8]
    adds r1, #1
    str r1, [r10, #8]
command:
    ldr r7, [r10, #0x10]
    cmp r7, #0
    beq loop
    movs r0, #0x1e
    cmp r7, #1
    beq enable_route
    cmp r7, #2
    beq mask_route
    cmp r7, #3
    beq low_read
    cmp r7, #4
    beq high_read
    cmp r7, #5
    beq idle
    cmp r7, #6
    beq start
    cmp r7, #7
    beq status_read
    cmp r7, #9
    bhs accel_command
    movs r1, #0x64 /* command 8: threshold source, not DRDY ack */
    b read_byte
status_read:
    movs r1, #0x67
    b read_byte
low_read:
    movs r1, #0x68
    b read_byte
high_read:
    movs r1, #0x69
read_byte:
    movs r2, #1
    add r3, r10, #0x18
    bl motion_read
    b complete
idle:
    movs r1, #0x60
    movs r2, #0x8f
    b write_byte
start:
    movs r1, #0x60
    movs r2, #0x8c
    b write_byte
mask_route:
    movs r1, #0x62
    movs r2, #0
    b write_byte
enable_route:
    movs r1, #0x62
    movs r2, #1
write_byte:
    bl motion_write
    b complete
accel_command:
    movs r0, #0x19
    cmp r7, #9
    beq accel_enable
    cmp r7, #10
    beq accel_mask
    cmp r7, #11
    beq accel_start
    cmp r7, #12
    beq accel_low_x
    cmp r7, #13
    beq accel_high_x
    cmp r7, #14
    beq accel_high_y
    cmp r7, #15
    beq accel_high_z
    cmp r7, #16
    beq accel_active_low
    cmp r7, #17
    beq accel_active_high
    cmp r7, #18
    beq accel_xy
    cmp r7, #19
    beq accel_idle
    movs r1, #0x27 /* command 20 reads acceleration status */
    b read_byte
accel_enable:
    movs r1, #0x22
    movs r2, #0x10
    b write_byte
accel_mask:
    movs r1, #0x22
    movs r2, #0
    b write_byte
accel_start:
    movs r1, #0x20
    movs r2, #0x57
    b write_byte
accel_xy:
    movs r1, #0x20
    movs r2, #0x53
    b write_byte
accel_idle:
    movs r1, #0x20
    movs r2, #7
    b write_byte
accel_low_x:
    movs r1, #0x28
    b read_byte
accel_high_x:
    movs r1, #0x29
    b read_byte
accel_high_y:
    movs r1, #0x2b
    b read_byte
accel_high_z:
    movs r1, #0x2d
    b read_byte
accel_active_low:
    movs r1, #0x25
    movs r2, #2
    b write_byte
accel_active_high:
    movs r1, #0x25
    movs r2, #0
    b write_byte
complete:
    str r7, [r10, #0x14]
    movs r0, #0
    str r0, [r10, #0x10]
    b loop
.include "twim-register.inc"
