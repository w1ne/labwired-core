/*
 * LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 * SPDX-License-Identifier: MIT
 *
 * EK-RX261 (R5F52618BGFP) bare-metal S12AD proof: convert AN000 (P40) and
 * AN001 (P41) in single scan mode, then print both 12-bit codes on SCI6 as
 * "N=xxx D=xxx" (hex), forever.
 *
 * The register sequence is the RX261 User's Manual: Hardware
 * (R01UH1045EJ0100) one:
 * - PRCR = A502h opens MSTPCRA (section 12), MSTPA17 = 0 starts S12AD
 *   (section 11);
 * - PWPR.B0WI = 0, PWPR.PFSWE = 1, then P40PFS/P41PFS.ASEL = 1 select the
 *   analog function (section 21); PORT4.PDR/PMR bits 0-1 cleared (input);
 * - ADANSA0 = 0003h selects AN000/AN001, ADCSR.ADCS = 00b single scan,
 *   ADCSR.ADST = 1 starts it; ADST reads 0 when the scan ends (section 40.3);
 * - ADDR0/ADDR1 hold the right-justified results (ADCER.ADRFMT = 0).
 * Register addresses are the iodefine.h ones (hal_renesas a279c14e196c,
 * r_bsp/mcu/rx261/register_access/gnuc/iodefine.h).
 *
 * No data or bss section: every variable is on the stack, so the start-up
 * code only sets the stack pointer.
 */

#include <stdint.h>

#define REG8(a) (*(volatile uint8_t *)(a))
#define REG16(a) (*(volatile uint16_t *)(a))
#define REG32(a) (*(volatile uint32_t *)(a))

#define SYSTEM_MSTPCRA REG32(0x00080010)
#define SYSTEM_PRCR REG16(0x000803FE)
#define MPC_PWPR REG8(0x0008C11F)
#define MPC_P40PFS REG8(0x0008C160)
#define MPC_P41PFS REG8(0x0008C161)
#define PORT4_PDR REG8(0x0008C004)
#define PORT4_PMR REG8(0x0008C064)

#define S12AD_BASE 0x00089000
#define S12AD_ADCSR REG16(S12AD_BASE + 0x00)
#define S12AD_ADANSA0 REG16(S12AD_BASE + 0x04)
#define S12AD_ADCER REG16(S12AD_BASE + 0x0E)
#define S12AD_ADDR(n) REG16(S12AD_BASE + 0x20 + 2 * (n))
#define ADCSR_ADST (1u << 15)

#define SCI6_BASE 0x0008A0C0
#define SCI6_SMR REG8(SCI6_BASE + 0)
#define SCI6_BRR REG8(SCI6_BASE + 1)
#define SCI6_SCR REG8(SCI6_BASE + 2)
#define SCI6_TDR REG8(SCI6_BASE + 3)
#define SCI6_SSR REG8(SCI6_BASE + 4)
#define SSR_TDRE (1u << 7)
#define SCR_TE (1u << 5)

static void putc_sci6(char c)
{
	while (!(SCI6_SSR & SSR_TDRE)) {
	}
	SCI6_TDR = (uint8_t)c;
}

static void put_hex3(uint16_t v)
{
	static const char digits[] = "0123456789ABCDEF";
	putc_sci6(digits[(v >> 8) & 0xF]);
	putc_sci6(digits[(v >> 4) & 0xF]);
	putc_sci6(digits[v & 0xF]);
}

int main(void)
{
	/* SCI6: asynchronous 8N1, PCLKB/1, BRR 16 = 115200 baud at 64 MHz
	 * (approximate; the model times the line from its own baud setting). */
	SCI6_SCR = 0;
	SCI6_SMR = 0;
	SCI6_BRR = 16;
	SCI6_SCR = SCR_TE;

	/* Release S12AD from module stop. */
	SYSTEM_PRCR = 0xA502;
	SYSTEM_MSTPCRA &= ~(1u << 17);
	SYSTEM_PRCR = 0xA500;

	/* P40/P41 as analog inputs. */
	PORT4_PDR &= (uint8_t)~0x03;
	PORT4_PMR &= (uint8_t)~0x03;
	MPC_PWPR = 0x00;
	MPC_PWPR = 0x40;
	MPC_P40PFS = 0x80;
	MPC_P41PFS = 0x80;
	MPC_PWPR = 0x00;
	MPC_PWPR = 0x80;

	S12AD_ADCER = 0;
	S12AD_ADANSA0 = 0x0003;
	for (;;) {
		S12AD_ADCSR = ADCSR_ADST;
		while (S12AD_ADCSR & ADCSR_ADST) {
		}
		uint16_t ntc = S12AD_ADDR(0);
		uint16_t diode = S12AD_ADDR(1);
		putc_sci6('N');
		putc_sci6('=');
		put_hex3(ntc);
		putc_sci6(' ');
		putc_sci6('D');
		putc_sci6('=');
		put_hex3(diode);
		putc_sci6('\r');
		putc_sci6('\n');
	}
}
