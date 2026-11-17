#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
#[cfg(feature = "stm32f429-tim2")]
use cortex_m_rt::exception;
use panic_halt as _;

#[cfg(feature = "stm32f429-m4f-fpu")]
mod m4f_fpu_smoke;

const RCC: u32 = 0x4002_3800;
const RCC_AHB1ENR: *mut u32 = (RCC + 0x30) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC + 0x40) as *mut u32;
const RCC_APB2ENR: *mut u32 = (RCC + 0x44) as *mut u32;

const GPIOA: u32 = 0x4002_0000;
const GPIOB: u32 = 0x4002_0400;
const GPIOD: u32 = 0x4002_0C00;
const USART2: u32 = 0x4000_4400;

const MODER: u32 = 0x00;
const AFRL: u32 = 0x20;
const BSRR: u32 = 0x18;
const USART_SR: *const u32 = USART2 as *const u32;
const USART_DR: *mut u32 = (USART2 + 0x04) as *mut u32;
const USART_BRR: *mut u32 = (USART2 + 0x08) as *mut u32;
const USART_CR1: *mut u32 = (USART2 + 0x0C) as *mut u32;

const TXE: u32 = 1 << 7;
const LED: u32 = 1 << 12;

// RM0090: TIM2 is the 32-bit general-purpose timer at 0x4000_0000, clocked
// from APB1 through RCC_APB1ENR.TIM2EN and routed to NVIC interrupt 28.
#[cfg(feature = "stm32f429-tim2")]
const TIM2: u32 = 0x4000_0000;
#[cfg(feature = "stm32f429-tim2")]
const TIM_CR1: *mut u32 = TIM2 as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM_DIER: *mut u32 = (TIM2 + 0x0C) as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM_SR: *mut u32 = (TIM2 + 0x10) as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM_EGR: *mut u32 = (TIM2 + 0x14) as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM_CNT: *mut u32 = (TIM2 + 0x24) as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM_PSC: *mut u32 = (TIM2 + 0x28) as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM_ARR: *mut u32 = (TIM2 + 0x2C) as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM_CEN: u32 = 1;
#[cfg(feature = "stm32f429-tim2")]
const TIM_UIF: u32 = 1;
#[cfg(feature = "stm32f429-tim2")]
const TIM2_IRQ: i16 = 28;
#[cfg(feature = "stm32f429-tim2")]
const TIM2_IRQ_BIT: u32 = 1 << TIM2_IRQ;
#[cfg(feature = "stm32f429-tim2")]
const TIM2_NVIC_ISER0: *mut u32 = 0xE000_E100 as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM2_NVIC_ICER0: *mut u32 = 0xE000_E180 as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM2_NVIC_ISPR0: *mut u32 = 0xE000_E200 as *mut u32;
#[cfg(feature = "stm32f429-tim2")]
const TIM2_NVIC_ICPR0: *mut u32 = 0xE000_E280 as *mut u32;

#[cfg(feature = "stm32f429-spi1")]
const SPI1: u32 = 0x4001_3000;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_CR1: *mut u32 = SPI1 as *mut u32;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_CR2: *mut u32 = (SPI1 + 0x04) as *mut u32;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_SR: *const u32 = (SPI1 + 0x08) as *const u32;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_DR: *mut u32 = (SPI1 + 0x0C) as *mut u32;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_IRQ: u32 = 35;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_IRQ_BIT: u32 = 1 << (SPI1_IRQ % 32);
#[cfg(feature = "stm32f429-spi1")]
const SPI1_NVIC_ISPR1: *const u32 = 0xE000_E204 as *const u32;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_NVIC_ICPR1: *mut u32 = 0xE000_E284 as *mut u32;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_RXNE: u32 = 1;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_TXE: u32 = 1 << 1;
#[cfg(feature = "stm32f429-spi1")]
const SPI1_BSY: u32 = 1 << 7;

#[cfg(feature = "stm32f429-i2c1")]
const I2C1: u32 = 0x4000_5400;
#[cfg(feature = "stm32f429-i2c1")]
const I2C1_CR1: *mut u32 = I2C1 as *mut u32;
#[cfg(feature = "stm32f429-i2c1")]
const I2C1_CR2: *mut u32 = (I2C1 + 0x04) as *mut u32;
#[cfg(feature = "stm32f429-i2c1")]
const I2C1_OAR1: *mut u32 = (I2C1 + 0x08) as *mut u32;
#[cfg(feature = "stm32f429-i2c1")]
const I2C1_DR: *mut u32 = (I2C1 + 0x10) as *mut u32;
#[cfg(feature = "stm32f429-i2c1")]
const I2C1_SR1: *const u32 = (I2C1 + 0x14) as *const u32;
#[cfg(feature = "stm32f429-i2c1")]
const I2C1_SR2: *const u32 = (I2C1 + 0x18) as *const u32;
#[cfg(feature = "stm32f429-i2c1")]
const I2C1_CCR: *mut u32 = (I2C1 + 0x1C) as *mut u32;
#[cfg(feature = "stm32f429-i2c1")]
const I2C1_TRISE: *mut u32 = (I2C1 + 0x20) as *mut u32;

#[cfg(feature = "stm32f429-tim2")]
static mut TIM2_IRQ_RECORD: [u32; 2] = [0; 2];

#[cfg(feature = "stm32f429-tim2")]
#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    let record = core::ptr::addr_of_mut!(TIM2_IRQ_RECORD).cast::<u32>();
    if irqn == TIM2_IRQ {
        // TIMx_SR is rc_w0: clearing UIF releases the level interrupt source.
        write_volatile(TIM_SR, !TIM_UIF);
        write_volatile(record, read_volatile(record).wrapping_add(1));
    } else {
        write_volatile(record.add(1), read_volatile(record.add(1)).wrapping_add(1));
    }
}

#[cfg(feature = "stm32f429-tim2")]
fn smoke_tim2() -> bool {
    let record = unsafe { core::ptr::addr_of_mut!(TIM2_IRQ_RECORD).cast::<u32>() };
    unsafe {
        // TIM2EN is clear after reset; a gated write must not reach the timer.
        write_volatile(TIM_ARR, 0x1234);
    }
    let gated = unsafe { read_volatile(TIM_ARR) } == 0;
    unsafe {
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | 1);
    }
    let arr_reset = unsafe { read_volatile(TIM_ARR) } == u32::MAX;
    unsafe {
        write_volatile(TIM_CNT, 0x1234_5678);
    }
    let width32 = unsafe { read_volatile(TIM_CNT) } == 0x1234_5678;

    // Load a short period, observe an update with UIE clear, and verify the
    // interrupt remains masked at the NVIC while UIF is latched.
    unsafe {
        write_volatile(TIM_PSC, 0);
        write_volatile(TIM_ARR, 31);
        write_volatile(TIM_EGR, 1);
    }
    let ug =
        unsafe { read_volatile(TIM_SR) } & TIM_UIF != 0 && unsafe { read_volatile(TIM_CNT) } == 0;
    unsafe {
        write_volatile(TIM_SR, !TIM_UIF);
        write_volatile(TIM2_NVIC_ICPR0, TIM2_IRQ_BIT);
        write_volatile(TIM_CR1, TIM_CEN);
    }
    let uif_cleared = unsafe { read_volatile(TIM_SR) } & TIM_UIF == 0;
    let start = unsafe { read_volatile(TIM_CNT) };
    let mut advanced = false;
    let mut overflowed = false;
    for _ in 0..20_000 {
        let now = unsafe { read_volatile(TIM_CNT) };
        advanced |= now != start && now <= 31;
        if unsafe { read_volatile(TIM_SR) } & TIM_UIF != 0 {
            overflowed = true;
            break;
        }
    }
    let masked = overflowed && unsafe { read_volatile(TIM2_NVIC_ISPR0) } & TIM2_IRQ_BIT == 0;

    // Enable the NVIC line and UIE; the default handler clears UIF and counts
    // two real timer update exceptions.
    unsafe {
        write_volatile(TIM_CR1, 0);
        write_volatile(TIM_SR, !TIM_UIF);
        write_volatile(record, 0);
        write_volatile(record.add(1), 0);
        write_volatile(TIM2_NVIC_ISER0, TIM2_IRQ_BIT);
        write_volatile(TIM_DIER, 1);
        write_volatile(TIM_CR1, TIM_CEN);
    }
    for _ in 0..100_000 {
        if unsafe { read_volatile(record) } >= 2 {
            break;
        }
    }
    unsafe {
        write_volatile(TIM_CR1, 0);
        write_volatile(TIM_DIER, 0);
    }
    let irqs = unsafe { read_volatile(record) };
    let stopped_at = unsafe { read_volatile(TIM_CNT) };
    for _ in 0..2_000 {
        let _ = unsafe { read_volatile(TIM_SR) };
    }
    let stopped =
        unsafe { read_volatile(TIM_CNT) } == stopped_at && unsafe { read_volatile(record) } == irqs;
    let stray = unsafe { read_volatile(record.add(1)) };
    unsafe {
        write_volatile(TIM2_NVIC_ICER0, TIM2_IRQ_BIT);
        write_volatile(TIM_SR, !TIM_UIF);
        write_volatile(TIM2_NVIC_ICPR0, TIM2_IRQ_BIT);
    }
    let released = unsafe { read_volatile(TIM2_NVIC_ISPR0) } & TIM2_IRQ_BIT == 0;
    let ok = gated
        && arr_reset
        && width32
        && ug
        && uif_cleared
        && advanced
        && masked
        && irqs == 2
        && stray == 0
        && stopped
        && released;
    send(b"tim2 gated=");
    send(if gated { b"1" } else { b"0" });
    send(b" arr_reset=");
    send(if arr_reset { b"1" } else { b"0" });
    send(b" w32=");
    send(if width32 { b"1" } else { b"0" });
    send(b" ug=");
    send(if ug { b"1" } else { b"0" });
    send(b" uif_clr=");
    send(if uif_cleared { b"1" } else { b"0" });
    send(b" run=");
    send(if advanced { b"1" } else { b"0" });
    send(b" masked=");
    send(if masked { b"1" } else { b"0" });
    send(b" irq=");
    send(&[b'0' + irqs.min(9) as u8]);
    send(b" stop=");
    send(if stopped { b"1" } else { b"0" });
    send(b" rel=");
    send(if released { b"1" } else { b"0" });
    send(if ok { b" ok\r\n" } else { b" FAIL\r\n" });
    ok
}

#[cfg(feature = "stm32f429-spi1")]
fn smoke_spi1() -> bool {
    unsafe {
        // A write while APB2 is gated must not reach SPI1.
        write_volatile(SPI1_CR1, 1 << 6);
    }
    let gated = unsafe { read_volatile(SPI1_CR1) == 0 };
    unsafe {
        write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | (1 << 12));
    }
    let clock = unsafe { read_volatile(RCC_APB2ENR) & (1 << 12) != 0 };

    // PA4 is the GPIO chip select; PA5..PA7 are SPI1 SCK/MISO/MOSI (AF5).
    unsafe {
        register_rmw(GPIOA + MODER, 0b11 << (4 * 2), 0b01 << (4 * 2));
        write_volatile((GPIOA + BSRR) as *mut u32, 1 << 4); // deselect
        for pin in [5u32, 6, 7] {
            register_rmw(GPIOA + MODER, 0b11 << (pin * 2), 0b10 << (pin * 2));
            register_rmw(GPIOA + AFRL, 0xF << (pin * 4), 5 << (pin * 4));
        }

        // Master, software NSS, fPCLK/8, mode 0, 8-bit frames.
        write_volatile(SPI1_CR1, (1 << 9) | (1 << 8) | (0b010 << 3) | (1 << 2));
        write_volatile(SPI1_CR1, read_volatile(SPI1_CR1) | (1 << 6)); // SPE
        write_volatile(SPI1_NVIC_ICPR1, SPI1_IRQ_BIT);
        write_volatile(SPI1_CR2, 1 << 7); // TXEIE
    }
    let txe_pended = unsafe { read_volatile(SPI1_NVIC_ISPR1) & SPI1_IRQ_BIT != 0 };
    unsafe {
        write_volatile(SPI1_CR2, 0);
        write_volatile(SPI1_NVIC_ICPR1, SPI1_IRQ_BIT);
    }
    let txe_released = unsafe { read_volatile(SPI1_NVIC_ISPR1) & SPI1_IRQ_BIT == 0 };

    unsafe {
        write_volatile(SPI1_CR2, 1 << 6); // RXNEIE
        write_volatile((GPIOA + BSRR) as *mut u32, 1 << (4 + 16)); // select PA4-low slave
    }
    let mut frame = 0u32;
    let mut rxne_pended = false;
    let mut transfer_ok = true;
    for _ in 0..4 {
        if !wait_spi1(SPI1_SR, SPI1_TXE, SPI1_TXE) {
            transfer_ok = false;
            break;
        }
        unsafe { write_volatile(SPI1_DR, 0) };
        if !wait_spi1(SPI1_SR, SPI1_RXNE, SPI1_RXNE) {
            transfer_ok = false;
            break;
        }
        let rxne = unsafe { read_volatile(SPI1_SR) } & SPI1_RXNE != 0;
        let pending = unsafe { read_volatile(SPI1_NVIC_ISPR1) } & SPI1_IRQ_BIT != 0;
        rxne_pended |= rxne && pending;
        frame = (frame << 8) | (unsafe { read_volatile(SPI1_DR) } & 0xFF);
    }
    let idle = wait_spi1(SPI1_SR, SPI1_BSY, 0);
    unsafe { write_volatile((GPIOA + BSRR) as *mut u32, 1 << 4) }; // deselect
    let rxne_released = unsafe {
        read_volatile(SPI1_NVIC_ISPR1) & SPI1_IRQ_BIT == 0
            && read_volatile(SPI1_SR) & SPI1_RXNE == 0
    };
    unsafe { write_volatile(SPI1_CR2, 0) };

    let ok = gated
        && clock
        && transfer_ok
        && idle
        && frame == 0x0190_1600
        && txe_pended
        && txe_released
        && rxne_pended
        && rxne_released;
    send(b"spi1 max31855=");
    send_hex(frame);
    send(b" gated=");
    send(if gated { b"1" } else { b"0" });
    send(b" clock=");
    send(if clock { b"1" } else { b"0" });
    send(b" txeirq=");
    send(if txe_pended { b"1" } else { b"0" });
    send(b" rearm=");
    send(if txe_released { b"0" } else { b"1" });
    send(b" rxneirq=");
    send(if rxne_pended { b"1" } else { b"0" });
    send(b" released=");
    send(if rxne_released { b"1" } else { b"0" });
    send(if ok { b" ok\r\n" } else { b" FAIL\r\n" });
    ok
}

#[cfg(feature = "stm32f429-spi1")]
fn wait_spi1(address: *const u32, mask: u32, want: u32) -> bool {
    for _ in 0..100_000 {
        if unsafe { read_volatile(address) } & mask == want {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

#[cfg(feature = "stm32f429-i2c1")]
fn wait_i2c1(mask: u32, want: u32) -> bool {
    for _ in 0..100_000 {
        if unsafe { read_volatile(I2C1_SR1) } & mask == want {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

#[cfg(feature = "stm32f429-i2c1")]
fn clear_i2c1_addr() {
    // The legacy F4 controller clears ADDR after the SR1-then-SR2 read pair.
    unsafe {
        let _ = read_volatile(I2C1_SR1);
        let _ = read_volatile(I2C1_SR2);
    }
}

/// Read BME280's chip ID over the imported F429's legacy I2C1 controller.
#[cfg(feature = "stm32f429-i2c1")]
fn smoke_i2c1() -> bool {
    const I2C1EN: u32 = 1 << 21;
    const BME280_ADDRESS: u8 = 0x76;

    let (gated, clock) = unsafe {
        // A CR1 write while APB1 is gated must not reach the peripheral.
        write_volatile(I2C1_CR1, 1);
        let gated = read_volatile(I2C1_CR1) == 0;
        write_volatile(RCC_AHB1ENR, read_volatile(RCC_AHB1ENR) | (1 << 1));
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | I2C1EN);
        let clock = read_volatile(RCC_APB1ENR) & I2C1EN != 0;

        // PB6/PB7 are I2C1 SCL/SDA on AF4; real-board open-drain setup.
        register_rmw(GPIOB + MODER, 0xF << 12, 0xA << 12);
        register_rmw(GPIOB + AFRL, 0xFF << 24, 0x44 << 24);
        register_rmw(GPIOB + 0x04, (1 << 6) | (1 << 7), (1 << 6) | (1 << 7));

        // FREQ=16 MHz, standard-mode 100 kHz, and the RM0090 OAR1 reserved bit.
        write_volatile(I2C1_CR2, 16);
        write_volatile(I2C1_CCR, 80);
        write_volatile(I2C1_TRISE, 17);
        write_volatile(I2C1_OAR1, 1 << 14);
        write_volatile(I2C1_CR1, 1); // PE; ACK stays clear for a one-byte read.

        // Select BME280 register 0xD0, then repeated-START into a one-byte read.
        write_volatile(I2C1_CR1, 1 | (1 << 8));
        (gated, clock)
    };
    let start = wait_i2c1(1 << 0, 1 << 0);
    if !start {
        send(b"i2c1 bme280_id=00000000 gated=0 clock=0 FAIL\r\n");
        return false;
    }
    unsafe { write_volatile(I2C1_DR, u32::from(BME280_ADDRESS << 1)) };
    if !wait_i2c1(1 << 1, 1 << 1) {
        return false;
    }
    clear_i2c1_addr();
    unsafe { write_volatile(I2C1_DR, 0xD0) };
    if !wait_i2c1(1 << 2, 1 << 2) {
        return false;
    }

    register_rmw(I2C1, 0, 1 << 8);
    if !wait_i2c1(1 << 0, 1 << 0) {
        return false;
    }
    unsafe { write_volatile(I2C1_DR, u32::from((BME280_ADDRESS << 1) | 1)) };
    if !wait_i2c1(1 << 1, 1 << 1) {
        return false;
    }
    clear_i2c1_addr();
    register_rmw(I2C1, 0, 1 << 9); // STOP after the byte.
    if !wait_i2c1(1 << 6, 1 << 6) {
        return false;
    }
    let chip_id = unsafe { read_volatile(I2C1_DR) & 0xFF };
    let mut idle = false;
    for _ in 0..100_000 {
        if unsafe { read_volatile(I2C1_SR2) } & 0x0002 == 0 {
            idle = true;
            break;
        }
    }

    let ok = gated && clock && start && idle && chip_id == 0x60;
    send(b"i2c1 bme280_id=");
    send_hex(chip_id);
    send(b" gated=");
    send(if gated { b"1" } else { b"0" });
    send(b" clock=");
    send(if clock { b"1" } else { b"0" });
    send(if ok { b" ok\r\n" } else { b" FAIL\r\n" });
    ok
}

const SYSTICK_CSR: u32 = 0xE000_E010;
const SYSTICK_RVR: u32 = 0xE000_E014;
const SCB_ICSR: u32 = 0xE000_ED04;
const SCB_VTOR: u32 = 0xE000_ED08;
const SCB_SHPR3: u32 = 0xE000_ED20;
const ICSR_PENDSTSET: u32 = 1 << 26;
const ICSR_PENDSTCLR: u32 = 1 << 25;
const ICSR_PENDSVSET: u32 = 1 << 28;
const ICSR_PENDSVCLR: u32 = 1 << 27;

// These scratch locations are in the low RAM region, away from the runtime's
// descending MSP stack at the top of SRAM.
const TEST_VECTOR_TABLE: u32 = 0x2000_0080;
const PSP_TAILCHAIN_RECORD: u32 = 0x2000_0100;
const PSP_TAILCHAIN_STACK_TOP: u32 = 0x2000_0200;
const THREAD_R0_MARKER: u32 = 0x5448_5244;
const HANDLER_R0_MARKER: u32 = 0x5045_4E44;

fn register_rmw(address: u32, clear: u32, set: u32) {
    unsafe {
        let register = address as *mut u32;
        write_volatile(register, (read_volatile(register) & !clear) | set);
    }
}

fn configure_usart2_pins() {
    for pin in [2u32, 3] {
        register_rmw(GPIOA + MODER, 0b11 << (pin * 2), 0b10 << (pin * 2));
        register_rmw(GPIOA + AFRL, 0xF << (pin * 4), 7 << (pin * 4));
    }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART_SR) & TXE == 0 {}
        write_volatile(USART_DR, byte as u32);
    }
}

fn send(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

fn set_led(on: bool) {
    let bit = if on { LED } else { LED << 16 };
    unsafe { write_volatile((GPIOD + BSRR) as *mut u32, bit) };
}

/// Convert ADC1 channel 1 from the 25% PA1 test fixture. At 0.825 V against
/// a 3.3 V reference, the STM32F4 12-bit result is floor(0.25 * 4095) = 0x03FF.
#[cfg(any(feature = "stm32f429-adc1", feature = "stm32f412-adc1"))]
fn smoke_adc1() -> bool {
    const ADC1: u32 = 0x4001_2000;
    const ADC_SR: *const u32 = ADC1 as *const u32;
    const ADC_CR2: *mut u32 = (ADC1 + 0x08) as *mut u32;
    const ADC_SQR3: *mut u32 = (ADC1 + 0x34) as *mut u32;
    const ADC_DR: *const u32 = (ADC1 + 0x4C) as *const u32;
    const ADC1EN: u32 = 1 << 8;
    const ADON: u32 = 1;
    const SWSTART: u32 = 1 << 30;
    const EOC: u32 = 1 << 1;

    unsafe {
        // A write while APB2 is gated must be ignored.
        write_volatile(ADC_CR2, ADON);
        let gated = read_volatile(ADC_CR2) == 0;
        write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | ADC1EN);
        let clock = read_volatile(RCC_APB2ENR) & ADC1EN != 0;

        // PA1 analog mode and the first regular sequence slot select IN1.
        register_rmw(GPIOA + MODER, 0b11 << 2, 0b11 << 2);
        write_volatile(ADC_SQR3, 1);
        write_volatile(ADC_CR2, ADON);
        write_volatile(ADC_CR2, ADON | SWSTART);
        let mut eoc = false;
        for _ in 0..100_000 {
            if read_volatile(ADC_SR) & EOC != 0 {
                eoc = true;
                break;
            }
            core::hint::spin_loop();
        }
        let code = read_volatile(ADC_DR) & 0x0FFF;
        write_volatile(ADC_CR2, 0);

        send(b"adc1 in1=");
        send_hex(code);
        send(b" gated=");
        send(if gated { b"1" } else { b"0" });
        send(b" clock=");
        send(if clock { b"1" } else { b"0" });
        send(b" eoc=");
        send(if eoc { b"1" } else { b"0" });
        let passed = gated && clock && eoc && code == 0x03FF;
        send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
        passed
    }
}

fn send_hex(value: u32) {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    for shift in [28, 24, 20, 16, 12, 8, 4, 0] {
        putc(DIGITS[((value >> shift) & 0xF) as usize]);
    }
}

fn delay() {
    for _ in 0..40_000 {
        unsafe {
            let _ = read_volatile((GPIOD + 0x14) as *const u32);
        }
    }
}

/// Verify that a SysTick exception entered from Thread mode on PSP tail-chains
/// to the lower-priority PendSV handler without replacing its exception frame.
fn smoke_systick_pendsv_psp_tailchain() -> bool {
    let prior_primask: u32;
    let prior_control: u32;
    let prior_psp: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "mrs {control}, CONTROL",
            "mrs {psp}, PSP",
            "cpsid i",
            primask = out(reg) prior_primask,
            control = out(reg) prior_control,
            psp = out(reg) prior_psp,
            options(nomem, nostack, preserves_flags),
        );
    }
    let prior_msp: u32;
    unsafe {
        core::arch::asm!(
            "mrs {}, MSP",
            out(reg) prior_msp,
            options(nomem, nostack, preserves_flags),
        );
    }

    let prior_vtor = unsafe { read_volatile(SCB_VTOR as *const u32) };
    let prior_shpr3 = unsafe { read_volatile(SCB_SHPR3 as *const u32) };
    let prior_rvr = unsafe { read_volatile(SYSTICK_RVR as *const u32) };
    let prior_csr = unsafe { read_volatile(SYSTICK_CSR as *const u32) } & 0x7;

    unsafe {
        write_volatile(SYSTICK_CSR as *mut u32, 0);
        write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
        // SysTick outranks PendSV; leave the reserved SHPR3 bytes untouched.
        write_volatile(
            SCB_SHPR3 as *mut u32,
            (prior_shpr3 & 0x0000_FFFF) | 0x80C0_0000,
        );
        for slot in 0..16 {
            write_volatile((TEST_VECTOR_TABLE + slot * 4) as *mut u32, 0);
        }
        write_volatile(
            (TEST_VECTOR_TABLE + 14 * 4) as *mut u32,
            (psp_tailchain_pendsv_handler as *const () as usize as u32) | 1,
        );
        write_volatile(
            (TEST_VECTOR_TABLE + 15 * 4) as *mut u32,
            (psp_tailchain_systick_handler as *const () as usize as u32) | 1,
        );
        for word in 0..17 {
            write_volatile((PSP_TAILCHAIN_RECORD + word * 4) as *mut u32, 0);
        }
        write_volatile(SCB_VTOR as *mut u32, TEST_VECTOR_TABLE);
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    if prior_primask == 0 {
        unsafe { pend_systick_from_psp() };
    }

    let record = PSP_TAILCHAIN_RECORD;
    let parent_ipsr = unsafe { read_volatile(record as *const u32) };
    let parent_icsr_before = unsafe { read_volatile((record + 4) as *const u32) };
    let parent_icsr_after = unsafe { read_volatile((record + 8) as *const u32) };
    let parent_msp = unsafe { read_volatile((record + 12) as *const u32) };
    let parent_exc_return = unsafe { read_volatile((record + 16) as *const u32) };
    let thread_r0 = unsafe { read_volatile((record + 20) as *const u32) };
    let child_ipsr = unsafe { read_volatile((record + 24) as *const u32) };
    let child_msp = unsafe { read_volatile((record + 28) as *const u32) };
    let child_exc_return = unsafe { read_volatile((record + 32) as *const u32) };
    let child_r0 = unsafe { read_volatile((record + 36) as *const u32) };
    let child_icsr = unsafe { read_volatile((record + 40) as *const u32) };
    let child_hit = unsafe { read_volatile((record + 44) as *const u32) == 1 };
    let parent_psp = unsafe { read_volatile((record + 48) as *const u32) };
    let child_psp = unsafe { read_volatile((record + 52) as *const u32) };
    let wrapper_msp = unsafe { read_volatile((record + 56) as *const u32) };
    let wrapper_psp = unsafe { read_volatile((record + 60) as *const u32) };
    let wrapper_control = unsafe { read_volatile((record + 64) as *const u32) };

    let thread_ipsr: u32;
    let msp_after: u32;
    let psp_after: u32;
    let control_after: u32;
    unsafe {
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            "mrs {msp}, MSP",
            "mrs {psp}, PSP",
            "mrs {control}, CONTROL",
            ipsr = out(reg) thread_ipsr,
            msp = out(reg) msp_after,
            psp = out(reg) psp_after,
            control = out(reg) control_after,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
        write_volatile(SYSTICK_CSR as *mut u32, 0);
        write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
        write_volatile(SCB_VTOR as *mut u32, prior_vtor);
        write_volatile(SCB_SHPR3 as *mut u32, prior_shpr3);
        write_volatile(SYSTICK_RVR as *mut u32, prior_rvr);
        write_volatile(SYSTICK_CSR as *mut u32, prior_csr);
        core::arch::asm!(
            "msr PSP, {psp}",
            "msr CONTROL, {control}",
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            psp = in(reg) prior_psp,
            control = in(reg) prior_control,
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    let deferred = parent_ipsr == 15
        && parent_icsr_before & 0x1FF == 15
        && parent_icsr_before & ICSR_PENDSVSET == 0
        && parent_icsr_after & 0x1FF == 15
        && parent_icsr_after & ICSR_PENDSVSET != 0;
    let frame_reused = parent_msp == child_msp
        && parent_exc_return == 0xFFFF_FFFD
        && child_exc_return == parent_exc_return
        && parent_psp == child_psp
        && parent_psp == PSP_TAILCHAIN_STACK_TOP - 32
        && wrapper_msp == prior_msp
        && parent_msp == prior_msp - 16;
    let child_is_pendsv = child_hit
        && child_ipsr == 14
        && child_icsr & 0x1FF == 14
        && child_icsr & ICSR_PENDSVSET == 0;
    let r0_live =
        thread_r0 == THREAD_R0_MARKER && child_r0 == HANDLER_R0_MARKER && child_r0 != thread_r0;
    let cleanup = unsafe { read_volatile(SCB_VTOR as *const u32) == prior_vtor }
        && unsafe { read_volatile(SCB_SHPR3 as *const u32) == prior_shpr3 }
        && unsafe {
            read_volatile(SCB_ICSR as *const u32) & (ICSR_PENDSTSET | ICSR_PENDSVSET) == 0
        }
        && msp_after == prior_msp
        && psp_after == prior_psp
        && control_after == prior_control
        && wrapper_psp == prior_psp
        && wrapper_control == prior_control
        && thread_ipsr == 0;
    let passed = prior_primask == 0
        && prior_control & 2 == 0
        && deferred
        && frame_reused
        && child_is_pendsv
        && r0_live
        && cleanup;

    send(b"systick pendsv psp tailchain frame=");
    putc(if frame_reused { b'1' } else { b'0' });
    send(b" r0=");
    putc(if r0_live { b'1' } else { b'0' });
    send(b" pending=");
    putc(if deferred && child_is_pendsv && cleanup {
        b'1'
    } else {
        b'0'
    });
    send(b" ipsr=");
    for shift in (0..8).rev() {
        let digit = ((child_ipsr >> (shift * 4)) & 0xF) as u8;
        putc(if digit < 10 {
            b'0' + digit
        } else {
            b'A' + digit - 10
        });
    }
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Switch Thread mode to a dedicated PSP stack, then software-pend SysTick.
#[unsafe(naked)]
unsafe extern "C" fn pend_systick_from_psp() {
    core::arch::naked_asm!(
        "mrs r3, msp",
        "push {{r4, r5, r6, lr}}",
        "mrs r4, control",
        "mrs r5, psp",
        "ldr r0, =0x20000100",
        "str r3, [r0, #56]",
        "str r5, [r0, #60]",
        "str r4, [r0, #64]",
        "ldr r1, =0x20000200",
        "msr psp, r1",
        "movs r1, #2",
        "orrs r1, r4",
        "msr control, r1",
        "isb",
        "ldr r0, =0x54485244",
        "ldr r1, =0xE000ED04",
        "movs r2, #1",
        "lsls r2, r2, #26",
        "str r2, [r1]",
        "ldr r1, =0x20000100",
        "movw r6, #4096",
        "1:",
        "ldr r2, [r1, #44]",
        "cmp r2, #0",
        "bne 2f",
        "subs r6, r6, #1",
        "bne 1b",
        "2:",
        "msr psp, r5",
        "msr control, r4",
        "isb",
        "pop {{r4, r5, r6, pc}}",
    );
}

/// Record the PSP frame, pend PendSV, and leave a distinct live R0 marker.
#[unsafe(naked)]
unsafe extern "C" fn psp_tailchain_systick_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20000100",
        "mrs r3, ipsr",
        "str r3, [r0, #0]",
        "ldr r3, =0xE000ED04",
        "ldr r1, [r3]",
        "str r1, [r0, #4]",
        "movs r1, #1",
        "lsls r1, r1, #28",
        "str r1, [r3]",
        "ldr r1, [r3]",
        "str r1, [r0, #8]",
        "mrs r1, msp",
        "str r1, [r0, #12]",
        "mov r1, lr",
        "str r1, [r0, #16]",
        "mrs r1, psp",
        "str r1, [r0, #48]",
        "str r2, [r0, #20]",
        "ldr r0, =0x50454E44",
        "bx lr",
    );
}

/// Capture PendSV's reused PSP frame before it returns to Thread mode.
#[unsafe(naked)]
unsafe extern "C" fn psp_tailchain_pendsv_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20000100",
        "mrs r3, ipsr",
        "str r3, [r0, #24]",
        "mrs r3, msp",
        "str r3, [r0, #28]",
        "mov r3, lr",
        "str r3, [r0, #32]",
        "str r2, [r0, #36]",
        "ldr r3, =0xE000ED04",
        "ldr r3, [r3]",
        "str r3, [r0, #40]",
        "mrs r3, psp",
        "str r3, [r0, #52]",
        "movs r3, #1",
        "str r3, [r0, #44]",
        "bx lr",
    );
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA/GPIOD on AHB1 and USART2 on APB1.
        write_volatile(
            RCC_AHB1ENR,
            read_volatile(RCC_AHB1ENR) | (1 << 0) | (1 << 3),
        );
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));
    }

    configure_usart2_pins();
    register_rmw(GPIOD + MODER, 0b11 << 24, 0b01 << 24);
    unsafe {
        // The smoke checks the register path, not baud accuracy or a clock tree.
        write_volatile(USART_BRR, 0x8B);
        write_volatile(USART_CR1, (1 << 13) | (1 << 3)); // UE | TE
    }

    set_led(false);
    send(b"STM32F4 shared profile smoke\n");

    #[cfg(feature = "stm32f429-tim2")]
    let _tim2_ok = smoke_tim2();

    let psp_tailchain_ok = smoke_systick_pendsv_psp_tailchain();
    send(if psp_tailchain_ok {
        b"STM32F429 PSP tailchain PASS\r\n"
    } else {
        b"STM32F429 PSP tailchain FAIL\r\n"
    });

    #[cfg(feature = "stm32f429-m4f-fpu")]
    m4f_fpu_smoke::run();

    #[cfg(any(feature = "stm32f429-adc1", feature = "stm32f412-adc1"))]
    let _adc1_ok = smoke_adc1();

    #[cfg(feature = "stm32f429-spi1")]
    let _spi1_ok = smoke_spi1();

    #[cfg(feature = "stm32f429-i2c1")]
    let _i2c1_ok = smoke_i2c1();

    loop {
        set_led(true);
        delay();
        set_led(false);
        delay();
    }
}
