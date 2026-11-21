/* Classic ESP32 console fixture (register level, no ROM, no IDF).
 *
 * UART0 (0x3FF40000) prints `VCP_BOOT` / `VCP_DONE`. Between the console
 * bytes, UART2 (0x3FF6E000, Arduino `Serial2`) sends `AUX_UART2_DATA` one
 * byte at a time and waits for its TX FIFO to drain (STATUS.TXFIFO_CNT,
 * bits [23:16]), so an all-UART capture interleaves the two streams.
 */
    .section .data
con_boot: .ascii "VCP_BOOT\r\n"
aux:      .ascii "AUX_UART2_DATA\r\n"
con_done: .ascii "VCP_DONE\r\n"

    .section .text
    .literal_position
    .global _start
    .align 4
_start:
    movi    a2, 0x3FF40000          /* UART0 FIFO */
    movi    a3, 0x3FF6E000          /* UART2 FIFO */
    movi    a4, con_boot
    movi    a5, aux
    movi    a6, 10                  /* len(con_boot) */
1:  l8ui    a7, a4, 0
    s32i    a7, a2, 0
    call0   aux_byte
    addi    a4, a4, 1
    addi    a6, a6, -1
    bnez    a6, 1b
    movi    a6, 6                   /* rest of aux */
2:  call0   aux_byte
    addi    a6, a6, -1
    bnez    a6, 2b
    movi    a4, con_done
    movi    a6, 10
3:  l8ui    a7, a4, 0
    s32i    a7, a2, 0
    addi    a4, a4, 1
    addi    a6, a6, -1
    bnez    a6, 3b
4:  j       4b

    .align 4
aux_byte:                           /* send *a5++ on UART2, wait for drain */
    l8ui    a7, a5, 0
    s32i    a7, a3, 0
    addi    a5, a5, 1
5:  l32i    a8, a3, 0x1C
    extui   a8, a8, 16, 8
    bnez    a8, 5b
    ret
