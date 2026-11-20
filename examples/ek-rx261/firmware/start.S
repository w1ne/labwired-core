/* EK-RX261 bare-metal start-up: the fixed reset vector (0xFFFFFFFC) enters
 * here in supervisor mode on the interrupt stack (PSW.U = 0). Interrupts stay
 * disabled (PSW.I = 0 after reset); the firmware polls. */
	.section .text.start, "ax"
	.global _reset
_reset:
	mov.l	#__stack_top, r0
	bsr.a	_main
1:	bra.b	1b

	.section .fvectors, "a"
	.long	_reset
