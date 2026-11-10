#![no_std]
#![no_main]

//! STM32L151xD smoke for the imported Renode platform: USART1/2/3 (including
//! USART2 receiver enable and RXNE interrupt behavior), GPIOA..H,
//! SYSCFG EXTICR + EXTI, TIM2..TIM7 and TIM9..TIM11, SPI1..3, I2C1 and DMA1.
//! Register offsets, bases and IRQ numbers are ST's CMSIS `stm32l151xd.h`.
//!
//! The fixture system puts a MAX31855 on each SPI (thermocouple 25, 100 and
//! -25 °C, so each bus answers with its own frame), a second MAX31855 on SPI1
//! behind its own chip select (PB0, 50 °C) and a TMP102 at 0x48 on I2C1.
//!
//! Interrupts stay disabled in the NVIC: each check reads the line's
//! ISPR bit, which proves the block reaches the right NVIC input without a
//! device vector table. SPI checks also confirm the classic TXE/RXNE source
//! levels assert and release with their status and enable bits. Every check
//! prints its results and sets one bit of the summary word at `SUMMARY`.

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const SUMMARY: u32 = 0x2000_8000;

// USART (classic SR/DR/BRR/CR1 layout).
const USART1: u32 = 0x4001_3800;
const USART2: u32 = 0x4000_4400;
const USART3: u32 = 0x4000_4800;
const U_SR: u32 = 0x00;
const U_DR: u32 = 0x04;
const U_BRR: u32 = 0x08;
const U_CR1: u32 = 0x0C;
const USART_SR_TXE: u32 = 1 << 7;

// GPIO (AHB). The L1 address order puts H before F and G.
const GPIOA: u32 = 0x4002_0000;
const GPIOB: u32 = 0x4002_0400;
const GPIOC: u32 = 0x4002_0800;
const GPIOH: u32 = 0x4002_1400;
const GPIOF: u32 = 0x4002_1800;
const GPIOG: u32 = 0x4002_1C00;
const G_MODER: u32 = 0x00;
const G_OSPEEDR: u32 = 0x08;
const G_PUPDR: u32 = 0x0C;
const G_IDR: u32 = 0x10;
const G_ODR: u32 = 0x14;
const G_BSRR: u32 = 0x18;
const G_BRR: u32 = 0x28;

// SYSCFG / EXTI.
const SYSCFG_EXTICR1: u32 = 0x4001_0008;
const SYSCFG_EXTICR4: u32 = 0x4001_0014;
const EXTI: u32 = 0x4001_0400;
const E_IMR: u32 = 0x00;
const E_FTSR: u32 = 0x0C;
const E_SWIER: u32 = 0x10;
const E_PR: u32 = 0x14;
// SYSCFG_EXTICRx port codes (stm32l151xd.h SYSCFG_EXTICR1_EXTI0_Px).
const CODE_PC: u32 = 2;
const CODE_PH: u32 = 5;
const CODE_PG: u32 = 7;

// TIMx.
const T_CR1: u32 = 0x00;
const T_DIER: u32 = 0x0C;
const T_SR: u32 = 0x10;
const T_CNT: u32 = 0x24;
const T_PSC: u32 = 0x28;
const T_ARR: u32 = 0x2C;

// SPIx (classic CR1/CR2/SR/DR).
const S_CR1: u32 = 0x00;
const S_CR2: u32 = 0x04;
const S_SR: u32 = 0x08;
const S_DR: u32 = 0x0C;
const SPI_SR_RXNE: u32 = 1 << 0;
const SPI_SR_TXE: u32 = 1 << 1;
const SPI_SR_BSY: u32 = 1 << 7;
const SPI1: u32 = 0x4001_3000;

/// (name, base, NVIC line, CS port, CS pin, expected MAX31855 frame).
/// Frames: thermocouple << 18 (0.25 °C/LSB) | internal 22 °C << 4.
/// The second SPI1 slave (CS PB0): 50 °C thermocouple, 22 °C cold junction.
const SPI1_B_FRAME: u32 = 0x0320_1600;
const SPIS: [(&str, u32, u32, u32, u32, u32); 3] = [
    ("spi1", SPI1, 35, GPIOA, 4, 0x0190_1600),
    ("spi2", 0x4000_3800, 36, GPIOB, 12, 0x0640_1600),
    ("spi3", 0x4000_3C00, 47, GPIOA, 15, 0xFE70_1600),
];

// DMA1: ISR, IFCR, then channel n at 0x08 + 20 * (n - 1).
const DMA1: u32 = 0x4002_6000;
const D_ISR: u32 = 0x00;
const D_IFCR: u32 = 0x04;
const fn dma_ch(n: u32) -> u32 {
    DMA1 + 0x08 + 20 * (n - 1)
}
const C_CCR: u32 = 0x00;
const C_CNDTR: u32 = 0x04;
const C_CPAR: u32 = 0x08;
const C_CMAR: u32 = 0x0C;
const DMA_BUF: u32 = 0x2000_8100;

// I2C1 (F1/F4 "v1" IP).
const I2C1: u32 = 0x4000_5400;
const I_CR1: u32 = 0x00;
const I_CR2: u32 = 0x04;
const I_OAR1: u32 = 0x08;
const I_DR: u32 = 0x10;
const I_SR1: u32 = 0x14;
const I_SR2: u32 = 0x18;
const I_CCR: u32 = 0x1C;
const I_TRISE: u32 = 0x20;
const I2C_SR1_SB: u32 = 1 << 0;
const I2C_SR1_ADDR: u32 = 1 << 1;
const I2C_SR1_BTF: u32 = 1 << 2;
const I2C_SR1_RXNE: u32 = 1 << 6;
const I2C_SR1_AF: u32 = 1 << 10;
const I2C_CR1_START: u32 = 1 << 8;
const I2C_CR1_STOP: u32 = 1 << 9;
const I2C_CR1_ACK: u32 = 1 << 10;
const TMP102: u32 = 0x48;

const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;

/// (name, base, NVIC line, counter width in bits) — `stm32l151xd.h`
/// TIMx_BASE / TIMx_IRQn; widths from RM0038 (TIM5 is the 32-bit one).
const TIMERS: [(&str, u32, u32, u32); 9] = [
    ("tim2", 0x4000_0000, 28, 16),
    ("tim3", 0x4000_0400, 29, 16),
    ("tim4", 0x4000_0800, 30, 16),
    ("tim5", 0x4000_0C00, 46, 32),
    ("tim6", 0x4000_1000, 43, 16),
    ("tim7", 0x4000_1400, 44, 16),
    ("tim9", 0x4001_0800, 25, 16),
    ("tim10", 0x4001_0C00, 26, 16),
    ("tim11", 0x4001_1000, 27, 16),
];

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn nvic(reg: u32, irq: u32) -> (u32, u32) {
    (reg + 4 * (irq / 32), irq % 32)
}

fn pending(irq: u32) -> bool {
    let (ispr, b) = nvic(NVIC_ISPR0, irq);
    rd(ispr) & (1 << b) != 0
}

fn unpend(irq: u32) {
    let (icpr, b) = nvic(NVIC_ICPR0, irq);
    wr(icpr, 1 << b);
}

fn uart_init(base: u32) {
    wr(base + U_BRR, 0x45);
    wr(base + U_CR1, (1 << 13) | (1 << 3)); // UE | TE
}

fn send(base: u32, bytes: &[u8]) {
    for byte in bytes {
        let mut remaining = 100_000;
        while rd(base + U_SR) & USART_SR_TXE == 0 && remaining > 0 {
            remaining -= 1;
        }
        wr(base + U_DR, u32::from(*byte));
    }
}

/// USART2 receives two start-injected bytes only after both legacy CR1.UE and
/// CR1.RE are set. RXNEIE then gates IRQ 38 until the two-byte DR queue drains.
fn check_usart2_rx_gate() -> bool {
    const RXNE: u32 = 1 << 5;
    const RXNEIE: u32 = 1 << 5;
    const UE: u32 = 1 << 13;
    const RE: u32 = 1 << 2;
    const TE: u32 = 1 << 3;
    const IRQ: u32 = 38;
    const EXPECTED: u32 = 0x5253; // "RS"

    let mut re_off = true;
    for _ in 0..16 {
        re_off &= rd(USART2 + U_SR) & RXNE == 0;
    }
    wr(USART2 + U_CR1, RE | TE); // RE set, UE clear
    let mut ue_off = true;
    for _ in 0..16 {
        ue_off &= rd(USART2 + U_SR) & RXNE == 0;
    }

    unpend(IRQ);
    wr(USART2 + U_CR1, UE | RE | TE);
    let mut rxne_seen = false;
    for _ in 0..4096 {
        if rd(USART2 + U_SR) & RXNE != 0 {
            rxne_seen = true;
            break;
        }
    }
    let irq_isolated = rxne_seen && !pending(IRQ);
    wr(USART2 + U_CR1, UE | RE | TE | RXNEIE);
    let mut irq_asserted = false;
    for _ in 0..4096 {
        if pending(IRQ) {
            irq_asserted = true;
            break;
        }
    }

    let first = rd(USART2 + U_DR) & 0xFF;
    let rxne_after_first = rd(USART2 + U_SR) & RXNE != 0;
    let second = if rxne_after_first {
        rd(USART2 + U_DR) & 0xFF
    } else {
        0
    };
    let rxne_cleared = rd(USART2 + U_SR) & RXNE == 0;
    wr(USART2 + U_CR1, UE | TE); // keep USART2 TX available, disable its RX
    unpend(IRQ);
    let irq_released = !pending(IRQ);

    let data = (first << 8) | second;
    let ok = re_off
        && ue_off
        && irq_isolated
        && irq_asserted
        && data == EXPECTED
        && rxne_after_first
        && rxne_cleared
        && irq_released;
    puts("usart2 rx_gate re_off=");
    puts(if re_off { "1" } else { "0" });
    puts(" ue_off=");
    puts(if ue_off { "1" } else { "0" });
    puts(" data=");
    hex(data);
    puts(" rxne_after_first=");
    puts(if rxne_after_first { "1" } else { "0" });
    puts(" irq=");
    puts(if irq_isolated {
        "isolated"
    } else {
        "not_isolated"
    });
    puts(if irq_asserted {
        ",asserted"
    } else {
        ",not_asserted"
    });
    puts(if irq_released {
        ",released"
    } else {
        ",not_released"
    });
    puts(" ");
    verdict(ok);
    ok
}

fn puts(s: &str) {
    send(USART1, s.as_bytes());
}

fn flag(name: &str, ok: bool) {
    puts(name);
    puts(if ok { "=1 " } else { "=0 " });
}

fn verdict(ok: bool) {
    puts(if ok { "ok\r\n" } else { "FAIL\r\n" });
}

fn hex(v: u32) {
    let digits = b"0123456789ABCDEF";
    let mut buf = [0u8; 10];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        buf[2 + i] = digits[((v >> (28 - 4 * i)) & 0xF) as usize];
    }
    send(USART1, &buf);
}

/// RM0038 reset values of the debug-port pads, and set/reset through
/// BSRR/BRR on port H (the first port after E, at 0x40021400).
fn check_gpio() -> bool {
    let pa = rd(GPIOA + G_MODER) == 0xA800_0000 && rd(GPIOA + G_PUPDR) == 0x6400_0000;
    let pb = rd(GPIOB + G_MODER) == 0x0000_0280
        && rd(GPIOB + G_OSPEEDR) == 0x0000_00C0
        && rd(GPIOB + G_PUPDR) == 0x0000_0100;
    let pc = rd(GPIOC + G_MODER) == 0;
    // PH0 output; toggle it four times for the gpio_edges assertion.
    wr(GPIOH + G_MODER, 0b01);
    let mut toggles = true;
    for _ in 0..2 {
        wr(GPIOH + G_BSRR, 1);
        toggles &= rd(GPIOH + G_ODR) & 1 == 1 && rd(GPIOH + G_IDR) & 1 == 1;
        wr(GPIOH + G_BRR, 1);
        toggles &= rd(GPIOH + G_ODR) & 1 == 0;
    }
    // F and G are separate ports above H.
    wr(GPIOF + G_ODR, 0x00F0);
    wr(GPIOG + G_ODR, 0x0F00);
    let fg = rd(GPIOF + G_ODR) == 0x00F0 && rd(GPIOG + G_ODR) == 0x0F00 && rd(GPIOH + G_ODR) == 0;
    puts("gpio ");
    flag("pa", pa);
    flag("pb", pb);
    flag("pc", pc);
    flag("ph", toggles);
    flag("fg", fg);
    let ok = pa && pb && pc && toggles && fg;
    verdict(ok);
    ok
}

/// SWIER on lines 0, 7 and 12 reaches the grouped NVIC vectors
/// (EXTI0 = 6, EXTI9_5 = 23, EXTI15_10 = 40), and only while unmasked.
fn check_exti_swier() -> bool {
    wr(EXTI + E_IMR, 0);
    wr(EXTI + E_SWIER, 1 << 0);
    let masked = rd(EXTI + E_PR) & 1 == 1 && !pending(6);
    wr(EXTI + E_PR, 1);
    unpend(6);
    let mut routed = true;
    for &(line, irq) in &[(0u32, 6u32), (7, 23), (12, 40)] {
        wr(EXTI + E_IMR, 1 << line);
        wr(EXTI + E_SWIER, 1 << line);
        routed &= rd(EXTI + E_PR) & (1 << line) != 0 && pending(irq);
        wr(EXTI + E_PR, 1 << line);
        unpend(irq);
        routed &= rd(EXTI + E_PR) & (1 << line) == 0 && !pending(irq);
    }
    wr(EXTI + E_IMR, 0);
    puts("exti swier ");
    flag("masked", masked);
    flag("routed", routed);
    let ok = masked && routed;
    verdict(ok);
    ok
}

fn wait_level(port: u32, pin: u32, high: bool) -> bool {
    for _ in 0..2_000_000 {
        if (rd(port + G_IDR) >> pin) & 1 == u32::from(high) {
            return true;
        }
    }
    false
}

/// Falling edges from the two fixture buttons (PH1, PC13; pressed = low)
/// through the SYSCFG port mux. Line 1 first selects code 7 (PG on the L1):
/// the first PH1 press must not pend it. With code 5 (PH on the L1) the
/// second press must, on NVIC EXTI1 = 7. PC13 lands on EXTI15_10 = 40.
fn check_exti_pads() -> bool {
    // Inputs with pull-up.
    wr(GPIOH + G_PUPDR, 0b01 << 2);
    wr(GPIOC + G_PUPDR, 0b01 << 26);
    wr(SYSCFG_EXTICR1, CODE_PG << 4);
    wr(SYSCFG_EXTICR4, CODE_PC << 4);
    wr(EXTI + E_FTSR, (1 << 1) | (1 << 13));
    wr(EXTI + E_IMR, (1 << 1) | (1 << 13));

    let idle = wait_level(GPIOH, 1, true) && wait_level(GPIOC, 13, true);
    let press1 = wait_level(GPIOH, 1, false) && wait_level(GPIOH, 1, true);
    let other_port_quiet = rd(EXTI + E_PR) & (1 << 1) == 0 && !pending(7);

    wr(SYSCFG_EXTICR1, CODE_PH << 4);
    let press2 = wait_level(GPIOH, 1, false) && wait_level(GPIOH, 1, true);
    let ph = rd(EXTI + E_PR) & (1 << 1) != 0 && pending(7);
    wr(EXTI + E_PR, 1 << 1);
    unpend(7);

    let press3 = wait_level(GPIOC, 13, false) && wait_level(GPIOC, 13, true);
    let pc = rd(EXTI + E_PR) & (1 << 13) != 0 && pending(40);
    wr(EXTI + E_PR, 1 << 13);
    unpend(40);
    wr(EXTI + E_IMR, 0);

    puts("exti pads ");
    flag("idle", idle);
    flag("presses", press1 && press2 && press3);
    flag("pg_quiet", other_port_quiet);
    flag("ph1", ph);
    flag("pc13", pc);
    let ok = idle && press1 && press2 && press3 && other_port_quiet && ph && pc;
    verdict(ok);
    ok
}

/// Counter width (ARR keeps 16 or 32 bits), counting, and the update
/// interrupt reaching the timer's own NVIC line — and no other timer's.
fn check_timer(name: &str, base: u32, irq: u32, width: u32) -> bool {
    wr(base + T_ARR, 0xFFFF_FFFF);
    let top = if width == 32 { 0xFFFF_FFFF } else { 0xFFFF };
    let arr = rd(base + T_ARR);
    let width_ok = arr == top;

    wr(base + T_PSC, 0);
    wr(base + T_ARR, 999);
    wr(base + T_SR, 0);
    unpend(irq);
    wr(base + T_DIER, 1); // UIE
    wr(base + T_CR1, 1); // CEN
    let c0 = rd(base + T_CNT);
    let mut uif = false;
    for _ in 0..200_000 {
        if rd(base + T_SR) & 1 != 0 {
            uif = true;
            break;
        }
    }
    let counted = rd(base + T_CNT) != c0 || uif;
    let pend = pending(irq);
    // No other timer line is pending.
    let alone = TIMERS
        .iter()
        .all(|&(_, _, other, _)| other == irq || !pending(other));
    wr(base + T_CR1, 0);
    wr(base + T_DIER, 0);
    wr(base + T_SR, 0);
    unpend(irq);
    let released = !pending(irq);

    puts(name);
    puts(" arr=");
    hex(arr);
    puts(" ");
    flag("width", width_ok);
    flag("count", counted);
    flag("uif", uif);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = width_ok && counted && uif && pend && alone && released;
    verdict(ok);
    ok
}

fn wait(addr: u32, mask: u32, want: u32) -> bool {
    for _ in 0..100_000 {
        if rd(addr) & mask == want {
            return true;
        }
    }
    false
}

fn cs(port: u32, pin: u32, low: bool) {
    wr(port + G_BSRR, if low { 1 << (pin + 16) } else { 1 << pin });
}

fn cs_output(port: u32, pin: u32) {
    cs(port, pin, false);
    let moder = rd(port + G_MODER) & !(0b11 << (2 * pin));
    wr(port + G_MODER, moder | (0b01 << (2 * pin)));
}

/// Master, software NSS (SSM | SSI), fPCLK/8, mode 0, 8-bit frames.
fn spi_enable(base: u32) {
    wr(base + S_CR1, (1 << 9) | (1 << 8) | (0b010 << 3) | (1 << 2));
    wr(base + S_CR1, rd(base + S_CR1) | (1 << 6)); // SPE
}

fn spi_xfer(base: u32, out: u8) -> Option<u8> {
    if !wait(base + S_SR, SPI_SR_TXE, SPI_SR_TXE) {
        return None;
    }
    wr(base + S_DR, u32::from(out));
    if !wait(base + S_SR, SPI_SR_RXNE, SPI_SR_RXNE) {
        return None;
    }
    Some(rd(base + S_DR) as u8)
}

/// One 32-bit MAX31855 frame under CS, by polling.
fn max31855_frame(base: u32, port: u32, pin: u32) -> Option<u32> {
    cs(port, pin, true);
    let mut word = 0u32;
    for _ in 0..4 {
        match spi_xfer(base, 0) {
            Some(b) => word = (word << 8) | u32::from(b),
            None => {
                cs(port, pin, false);
                return None;
            }
        }
    }
    let idle = wait(base + S_SR, SPI_SR_BSY, 0);
    cs(port, pin, false);
    idle.then_some(word)
}

/// Like `max31855_frame`, but sample the RXNE IRQ while the received byte is
/// still in DR, then verify its level can fall when the read drains RXNE.
fn max31855_frame_with_irq(base: u32, irq: u32, port: u32, pin: u32) -> Option<(u32, bool)> {
    cs(port, pin, true);
    let mut word = 0u32;
    let mut irq_seen = false;
    for _ in 0..4 {
        if !wait(base + S_SR, SPI_SR_TXE, SPI_SR_TXE) {
            cs(port, pin, false);
            return None;
        }
        wr(base + S_DR, 0);
        if !wait(base + S_SR, SPI_SR_RXNE, SPI_SR_RXNE) {
            cs(port, pin, false);
            return None;
        }
        irq_seen |= pending(irq) && rd(base + S_SR) & SPI_SR_RXNE != 0;
        word = (word << 8) | (rd(base + S_DR) & 0xFF);
    }
    let idle = wait(base + S_SR, SPI_SR_BSY, 0);
    cs(port, pin, false);
    idle.then_some((word, irq_seen))
}

/// Each SPI reads its own device's frame, checks idle TXEIE and then RXNEIE;
/// status-and-enable levels must pend the SPI's own NVIC line and release it
/// as firmware disables TXEIE or drains RXNE.
fn check_spi(name: &str, base: u32, irq: u32, port: u32, pin: u32, want: u32) -> bool {
    cs_output(port, pin);
    spi_enable(base);
    let frame = max31855_frame(base, port, pin).unwrap_or(0xDEAD_BEEF);
    unpend(irq);
    let quiet = !pending(irq);

    // TXE is already high while the controller is idle. Enabling TXEIE must
    // assert the level immediately, and disabling it must drop that pend.
    wr(base + S_CR2, 1 << 7); // TXEIE
    let txe_irq = rd(base + S_SR) & SPI_SR_TXE != 0 && pending(irq);
    wr(base + S_CR2, 0);
    let txe_released = !pending(irq);

    wr(base + S_CR2, 1 << 6); // RXNEIE
    let again = max31855_frame_with_irq(base, irq, port, pin);
    let rx_irq = again.is_some_and(|(_, seen)| seen);
    let again_frame = again.map(|(frame, _)| frame);
    let data = frame == want && again_frame == Some(want);
    let pend = quiet && rx_irq;
    let alone = SPIS
        .iter()
        .all(|&(_, _, other, _, _, _)| other == irq || !pending(other));
    // Reading DR drained the RXNE source while RXNEIE stayed enabled.
    let released = !pending(irq) && rd(base + S_SR) & SPI_SR_RXNE == 0;
    wr(base + S_CR2, 0);

    puts(name);
    puts(" frame=");
    hex(frame);
    puts(" ");
    flag("data", data);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = data && pend && alone && released;
    verdict(ok);
    puts(name);
    puts(" TXEIE ");
    flag("pend", txe_irq);
    flag("released", txe_released);
    let txe_ok = txe_irq && txe_released;
    verdict(txe_ok);
    ok && txe_ok
}

/// Two MAX31855s share SPI1, told apart only by their GPIO chip selects
/// (PA4, PB0): each answers with its own frame while the other is released.
/// Then a frame cut short by CS↑ after two bytes: the next CS↓ must restart
/// the MAX31855 at bit 31, so the following whole frame is not rotated.
fn check_spi1_shared_cs() -> bool {
    let a = max31855_frame(SPI1, GPIOA, 4).unwrap_or(0xDEAD_BEEF);
    let b = max31855_frame(SPI1, GPIOB, 0).unwrap_or(0xDEAD_BEEF);
    let a_ok = a == SPIS[0].5;
    let b_ok = b == SPI1_B_FRAME;

    cs(GPIOA, 4, true);
    let _ = spi_xfer(SPI1, 0);
    let _ = spi_xfer(SPI1, 0);
    let _ = wait(SPI1 + S_SR, SPI_SR_BSY, 0);
    cs(GPIOA, 4, false);
    let after = max31855_frame(SPI1, GPIOA, 4).unwrap_or(0xDEAD_BEEF);
    let restart = after == SPIS[0].5;

    puts("spi1 shared a=");
    hex(a);
    puts(" b=");
    hex(b);
    puts(" after_cut=");
    hex(after);
    puts(" ");
    flag("a", a_ok);
    flag("b", b_ok);
    flag("restart", restart);
    let ok = a_ok && b_ok && restart;
    verdict(ok);
    ok
}

/// SPI1 full duplex through DMA1: channel 3 (SPI1_TX) feeds four zero
/// bytes, channel 2 (SPI1_RX) stores the frame. Only channel 2 has TCIE, so
/// only DMA1_Channel2_IRQn (12) pends — not 11, not 13.
fn check_dma_spi1() -> bool {
    let rx = DMA_BUF;
    let tx = DMA_BUF + 0x10;
    wr(rx, 0xFFFF_FFFF);
    wr(tx, 0);
    wr(DMA1 + D_IFCR, 0x0FFF_FFFF);
    for irq in 11..=17 {
        unpend(irq);
    }
    let ch2 = dma_ch(2);
    let ch3 = dma_ch(3);
    wr(ch2 + C_CPAR, SPI1 + S_DR);
    wr(ch2 + C_CMAR, rx);
    wr(ch2 + C_CNDTR, 4);
    wr(ch2 + C_CCR, (1 << 7) | (1 << 1) | 1); // MINC | TCIE | EN, P->M
    wr(ch3 + C_CPAR, SPI1 + S_DR);
    wr(ch3 + C_CMAR, tx);
    wr(ch3 + C_CNDTR, 4);
    wr(ch3 + C_CCR, (1 << 7) | (1 << 4) | 1); // MINC | DIR | EN, M->P

    cs(GPIOA, 4, true);
    wr(SPI1 + S_CR2, (1 << 1) | 1); // TXDMAEN | RXDMAEN
    let done = wait(DMA1 + D_ISR, 1 << 5, 1 << 5); // TCIF2
    let _ = wait(SPI1 + S_SR, SPI_SR_BSY, 0);
    cs(GPIOA, 4, false);
    wr(SPI1 + S_CR2, 0);

    let b = rd(rx).to_le_bytes();
    let frame = u32::from_be_bytes(b);
    let data = frame == SPIS[0].5;
    let isr = rd(DMA1 + D_ISR);
    // GIF | TCIF | HTIF (four items cross the half-way mark) on both channels.
    let flags = isr & 0xFF0 == 0x770 && rd(ch2 + C_CNDTR) == 0 && rd(ch3 + C_CNDTR) == 0;
    let pend = done && pending(12);
    let alone = !pending(11) && !pending(13) && (14..=17).all(|i| !pending(i));
    wr(DMA1 + D_IFCR, 0xFF0);
    unpend(12);
    let released = !pending(12) && rd(DMA1 + D_ISR) == 0;
    wr(ch2 + C_CCR, 0);
    wr(ch3 + C_CCR, 0);

    puts("dma1 spi1 frame=");
    hex(frame);
    puts(" ");
    flag("data", data);
    flag("flags", flags);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = data && flags && pend && alone && released;
    verdict(ok);
    ok
}

fn i2c_clear_addr() {
    let _ = rd(I2C1 + I_SR1);
    let _ = rd(I2C1 + I_SR2);
}

fn i2c_stop() -> bool {
    wr(I2C1 + I_CR1, rd(I2C1 + I_CR1) | I2C_CR1_STOP);
    wait(I2C1 + I_SR2, 0x0002, 0) // BUSY released
}

/// TMP102 temperature register (pointer 0): write pointer, repeated START,
/// two bytes with the final-byte NACK.
fn tmp102_read() -> Option<u16> {
    let sr1 = I2C1 + I_SR1;
    wr(I2C1 + I_CR1, rd(I2C1 + I_CR1) | I2C_CR1_START);
    if !wait(sr1, I2C_SR1_SB, I2C_SR1_SB) {
        return None;
    }
    wr(I2C1 + I_DR, TMP102 << 1);
    if !wait(sr1, I2C_SR1_ADDR, I2C_SR1_ADDR) {
        return None;
    }
    i2c_clear_addr();
    wr(I2C1 + I_DR, 0);
    if !wait(sr1, I2C_SR1_BTF, I2C_SR1_BTF) {
        return None;
    }
    wr(I2C1 + I_CR1, rd(I2C1 + I_CR1) | I2C_CR1_START | I2C_CR1_ACK);
    if !wait(sr1, I2C_SR1_SB, I2C_SR1_SB) {
        return None;
    }
    wr(I2C1 + I_DR, (TMP102 << 1) | 1);
    if !wait(sr1, I2C_SR1_ADDR, I2C_SR1_ADDR) {
        return None;
    }
    i2c_clear_addr();
    if !wait(sr1, I2C_SR1_RXNE, I2C_SR1_RXNE) {
        return None;
    }
    let msb = rd(I2C1 + I_DR) as u8;
    wr(I2C1 + I_CR1, rd(I2C1 + I_CR1) & !I2C_CR1_ACK);
    if !wait(sr1, I2C_SR1_RXNE, I2C_SR1_RXNE) {
        return None;
    }
    let lsb = rd(I2C1 + I_DR) as u8;
    i2c_stop().then(|| u16::from_be_bytes([msb, lsb]))
}

/// I2C1 master: OAR1 readback, a TMP102 read (25 °C = 0x1900), the event
/// line (ITEVTEN + SB -> 31) and an absent address NACKing onto the error
/// line (ITERREN + AF -> 32), not the event line.
fn check_i2c1() -> bool {
    wr(I2C1 + I_OAR1, (1 << 14) | 0xAA);
    let oar = rd(I2C1 + I_OAR1) & 0xFFFF == 0x40AA;
    wr(I2C1 + I_CR2, 16); // FREQ = 16 MHz
    wr(I2C1 + I_CCR, 80); // 100 kHz
    wr(I2C1 + I_TRISE, 17);
    wr(I2C1 + I_CR1, 1); // PE
    let temp = tmp102_read();
    let data = temp == Some(0x1900);

    unpend(31);
    unpend(32);
    wr(I2C1 + I_CR2, 16 | (1 << 9) | (1 << 8)); // ITEVTEN | ITERREN
    wr(I2C1 + I_CR1, rd(I2C1 + I_CR1) | I2C_CR1_START);
    let ev = wait(I2C1 + I_SR1, I2C_SR1_SB, I2C_SR1_SB) && pending(31);
    wr(I2C1 + I_DR, 0x50 << 1); // nothing at 0x50
    let af = wait(I2C1 + I_SR1, I2C_SR1_AF, I2C_SR1_AF);
    let er_pend = pending(32);
    unpend(31);
    let ev_quiet = !pending(31);
    let er = af && er_pend && ev_quiet;
    let stopped = i2c_stop();
    wr(I2C1 + I_SR1, !I2C_SR1_AF & 0xFFFF); // rc_w0
    unpend(32);
    let released = rd(I2C1 + I_SR1) & I2C_SR1_AF == 0 && !pending(32);
    wr(I2C1 + I_CR2, 16);

    puts("i2c1 tmp102=");
    hex(u32::from(temp.unwrap_or(0xFFFF)));
    puts(" ");
    flag("oar", oar);
    flag("data", data);
    flag("ev", ev);
    flag("af", af);
    flag("er_pend", er_pend);
    flag("ev_quiet", ev_quiet);
    flag("stop", stopped);
    flag("released", released);
    let ok = oar && data && ev && er && stopped && released;
    verdict(ok);
    ok
}

#[entry]
fn main() -> ! {
    uart_init(USART1);
    uart_init(USART2);
    uart_init(USART3);
    puts("STM32L151 Cortex-M3 smoke\r\n");
    let usart2_rx_gate_ok = check_usart2_rx_gate();
    send(USART2, b"usart2 ok\r\n");
    send(USART3, b"usart3 ok\r\n");

    let mut summary: u32 = 0;
    let mut mark = |bit: u32, ok: bool| {
        if ok {
            summary |= 1 << bit;
        }
    };
    mark(18, usart2_rx_gate_ok);
    mark(0, check_gpio());
    mark(1, check_exti_swier());
    for (i, &(name, base, irq, width)) in TIMERS.iter().enumerate() {
        mark(2 + i as u32, check_timer(name, base, irq, width));
    }
    mark(11, check_exti_pads());
    // Park the second SPI1 slave's CS high before SPI1 is used at all. A CS
    // pad that is not yet a GPIO output leaves its device on the bus (the
    // engine cannot tell an undriven pad from hardware NSS), so on SPI1 it
    // would answer over the PA4 device.
    cs_output(GPIOB, 0);
    for (i, &(name, base, irq, port, pin, want)) in SPIS.iter().enumerate() {
        mark(12 + i as u32, check_spi(name, base, irq, port, pin, want));
    }
    mark(15, check_dma_spi1());
    mark(16, check_i2c1());
    mark(17, check_spi1_shared_cs());
    let summary = summary | 0x1510_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
