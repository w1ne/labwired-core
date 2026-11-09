// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! What the STM32F76x/F77x adds to the F746 surface (RM0410, ST's CMSIS
//! `stm32f767xx.h`): the RCC enable masks, SYSCFG MEMRMP.SWP_FB, EXTI line
//! 24, the 2 MiB / 512 KiB geometry and SPI4..6. Shared by the stm32f777
//! fixture smoke (`main.rs`) and the NUCLEO-F767ZI board smoke
//! (`bin/nucleo_f767zi.rs`); the rest comes from `f746_checks`.

#![allow(dead_code)]

use crate::f746_checks::*;

/// Every APB1ENR / APB2ENR enable bit `stm32f767xx.h` defines.
pub const APB1_MASK_F767: u32 = 0xFFFF_EFFF;
pub const APB2_MASK_F767: u32 = 0x64F7_7FB3;
/// SPI4 (bit 13), SPI5 (bit 20), SPI6 (bit 21).
pub const APB2_SPI456: u32 = (1 << 13) | (1 << 20) | (1 << 21);

/// SRAM2 (16 KiB at 0x2007C000) is the top of the 512 KiB SRAM; the F746
/// ends at 0x2004FFFF.
pub const SRAM2: u32 = 0x2007_C000;
pub const SRAM_LAST: u32 = 0x2007_FFFC;
/// The last word of the 2 MiB flash; the F746 ends at 0x080FFFFF.
pub const FLASH_LAST: u32 = 0x081F_FFFC;

/// SPI4/5/6 (FIFO SPI): (name, base, NVIC line, CS port, CS pin, expected
/// MAX31855 frame). CS on each block's NSS pad (PE4, PF6, PG8) driven as a
/// GPIO. Frames: 60, 125 and 200 °C thermocouple, 22 °C cold junction.
pub const SPIS_F767: [(&str, u32, u32, u32, u32, u32); 3] = [
    ("spi4", 0x4001_3400, 84, gpio(4), 4, 0x03C0_1600),
    ("spi5", 0x4001_5000, 85, gpio(5), 6, 0x07D0_1600),
    ("spi6", 0x4001_5400, 86, gpio(6), 8, 0x0C80_1600),
];

/// `rcc_bringup` with the F767 masks: reset state, a holding clock gate,
/// and both the APB1ENR and APB2ENR write masks. Runs before anything else,
/// so USART1 is still gated and the result is reported later.
pub fn rcc_bringup_f767() -> (bool, bool, bool) {
    let reset = rd(RCC_CR) & 0b11 == 0b11 && rd(RCC_AHB1ENR) == AHB1_RESET;
    wr(GPIOD + G_ODR, 0x00A5);
    let gated = rd(GPIOD + G_ODR) == 0;
    wr(RCC_APB2ENR, 0xFFFF_FFFF);
    wr(RCC_APB1ENR, 0xFFFF_FFFF);
    let mask = rd(RCC_APB2ENR) == APB2_MASK_F767 && rd(RCC_APB1ENR) == APB1_MASK_F767;
    wr(RCC_AHB1ENR, AHB1_ON);
    wr(RCC_AHB2ENR, AHB2_ON);
    wr(RCC_APB2ENR, APB2_ON | APB2_SPI456);
    wr(RCC_APB1ENR, APB1_ON);
    wr(GPIOD + G_ODR, 0x00A5);
    let gate = gated && rd(GPIOD + G_ODR) == 0x00A5 && rd(RCC_AHB1ENR) == AHB1_ON;
    wr(GPIOD + G_ODR, 0);
    (reset, gate, mask)
}

/// SYSCFG on the F76x: MEMRMP keeps SWP_FB (bit 8) and SWP_FMC[11:10],
/// MEM_BOOT reads 0; PMC as on the F746; CMPCR.READY follows CMP_PD.
pub fn check_syscfg_f767() -> bool {
    wr(SYSCFG_MEMRMP, 0xFFFF_FFFF);
    let memrmp = rd(SYSCFG_MEMRMP);
    wr(SYSCFG_MEMRMP, 0);
    wr(SYSCFG_PMC, 0xFFFF_FFFF);
    let pmc = rd(SYSCFG_PMC);
    wr(SYSCFG_PMC, 0);
    let fields = memrmp == 0x0D00 && pmc == 0x0087_00FF;
    let idle = rd(SYSCFG_CMPCR) & (1 << 8) == 0;
    wr(SYSCFG_CMPCR, 1);
    let ready = idle && rd(SYSCFG_CMPCR) == 0x101;
    wr(SYSCFG_CMPCR, 0);
    puts("syscfg memrmp=");
    hex(memrmp);
    puts(" pmc=");
    hex(pmc);
    puts(" ");
    flag("fields", fields);
    flag("cmpcr", ready);
    let ok = fields && ready;
    verdict(ok);
    ok
}

/// EXTI has 25 lines on the F76x (line 24 = MDIO slave): IMR keeps bits
/// 0..24, and a software trigger on line 24 pends it.
pub fn check_exti_line24() -> bool {
    wr(EXTI + E_IMR, 0xFFFF_FFFF);
    let imr = rd(EXTI + E_IMR);
    wr(EXTI + E_IMR, 1 << 24);
    wr(EXTI + E_SWIER, 1 << 24);
    let pend = rd(EXTI + E_PR) & (1 << 24) != 0;
    wr(EXTI + E_PR, 1 << 24);
    let cleared = rd(EXTI + E_PR) & (1 << 24) == 0;
    wr(EXTI + E_IMR, 0);
    puts("exti lines imr=");
    hex(imr);
    puts(" ");
    flag("line24", pend && cleared);
    let ok = imr == 0x01FF_FFFF && pend && cleared;
    verdict(ok);
    ok
}

/// The 512 KiB SRAM and 2 MiB flash: SRAM2 and the last SRAM word hold
/// data, and the last flash word reads without a fault.
pub fn check_geometry() -> bool {
    wr(SRAM2, 0xC0DE_F767);
    wr(SRAM_LAST, 0x5A5A_A5A5);
    let sram = rd(SRAM2) == 0xC0DE_F767 && rd(SRAM_LAST) == 0x5A5A_A5A5;
    let flash = rd(FLASH_LAST);
    puts("geometry flash_last=");
    hex(flash);
    puts(" ");
    flag("sram2", sram);
    let ok = sram;
    verdict(ok);
    ok
}

/// SPI4/5/6 each send one frame with TXEIE (CR2 bit 7) set: TXE at the end
/// of the frame raises the SPI's own NVIC line and no other SPI's. The
/// board has no slaves on these buses, so nothing is read back.
///
/// The frame matters: the model raises TXEIE when a frame completes, not as
/// the TXE & TXEIE level silicon has (setting TXEIE while TXE is already 1
/// pends the line at once there). Both pend by the end of the frame.
pub fn check_spi456_lines() -> bool {
    let mut ok = true;
    puts("spi456 irq ");
    for &(name, base, irq, _, _, _) in &SPIS_F767 {
        unpend(irq);
        spi_enable(base);
        let quiet = !pending(irq);
        wr(base + S_CR2, SPI_CR2_8BIT | (1 << 7));
        wr8(base + S_DR, 0xA5);
        let sent = wait(base + S_SR, SPI_SR_BSY, 0) && rd(base + S_SR) & SPI_SR_TXE != 0;
        let pend = pending(irq);
        let alone = SPIS_F767
            .iter()
            .map(|s| s.2)
            .chain(SPIS.iter().map(|s| s.2))
            .all(|other| other == irq || !pending(other));
        wr(base + S_CR2, SPI_CR2_8BIT);
        unpend(irq);
        let released = !pending(irq);
        wr(base + S_CR1, 0);
        let this = quiet && sent && pend && alone && released;
        flag(name, this);
        ok &= this;
    }
    verdict(ok);
    ok
}
