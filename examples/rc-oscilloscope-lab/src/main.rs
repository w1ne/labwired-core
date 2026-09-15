#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

use cortex_m_rt::entry;
use panic_halt as _;

// STM32F401 register addresses (configs/chips/stm32f401.yaml).
const RCC_BASE: u32 = 0x4002_3800;
const RCC_AHB1ENR: *mut u32 = (RCC_BASE + 0x30) as *mut u32; // GPIOAEN = bit 0
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x40) as *mut u32; // TIM2EN = bit 0, USART2EN = bit 17
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x44) as *mut u32; // ADC1EN = bit 8

// GPIOA: MODER/OTYPER/OSPEEDR/PUPDR/IDR/ODR/BSRR/LCKR/AFRL/AFRH (RM0368 §8.4).
const GPIOA_BASE: u32 = 0x4002_0000;
const GPIOA_MODER: *mut u32 = (GPIOA_BASE + 0x00) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;

// TIM2: 32-bit general-purpose timer, free-running tick source for
// deterministic sampling (RM0368 §13).
const TIM2_BASE: u32 = 0x4000_0000;
const TIM2_CR1: *mut u32 = (TIM2_BASE + 0x00) as *mut u32;
const TIM2_SR: *mut u32 = (TIM2_BASE + 0x10) as *mut u32;
const TIM2_EGR: *mut u32 = (TIM2_BASE + 0x14) as *mut u32;
const TIM2_CNT: *const u32 = (TIM2_BASE + 0x24) as *const u32;
const TIM2_PSC: *mut u32 = (TIM2_BASE + 0x28) as *mut u32;
const TIM2_ARR: *mut u32 = (TIM2_BASE + 0x2C) as *mut u32;

// ADC1: F1-layout register set (SR/CR1/CR2/.../DR), gated by APB2ENR.ADC1EN.
const ADC1_BASE: u32 = 0x4001_2000;
const ADC1_SR: *mut u32 = (ADC1_BASE + 0x00) as *mut u32;
const ADC1_CR2: *mut u32 = (ADC1_BASE + 0x08) as *mut u32;
const ADC1_DR: *const u32 = (ADC1_BASE + 0x4C) as *const u32;

// USART2: F1-layout register set, TX on PA2 (AF7), debug console.
const USART2_BASE: u32 = 0x4000_4400;
const USART2_SR: *const u32 = (USART2_BASE + 0x00) as *const u32;
const USART2_DR: *mut u32 = (USART2_BASE + 0x04) as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x08) as *mut u32;
const USART2_CR1: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;

const SR_EOC: u32 = 1 << 1; // ADC1 SR: end of conversion
const SR_TXE: u32 = 1 << 7; // USART2 SR: TX empty

// PA5 toggles the RC network's step input; PA0 (ADC1 channel 0) samples the
// filtered output. Both timings are driven off the free-running TIM2 tick,
// not busy-wait loops, so the schedule is exact under simulation.
const TICK_HZ: u32 = 1_000_000; // TIM2 counts microseconds (PSC = cpu_hz/1MHz - 1)
const TOGGLE_PERIOD_US: u32 = 5_000; // 5 ms
const SAMPLE_PERIOD_US: u32 = 500; // 500 us
const VDD_MV: u32 = 3_300;
const ADC_FULL_SCALE: u32 = 4_095; // 12-bit

fn uart2_byte(byte: u8) {
    unsafe {
        for _ in 0..1_000 {
            if core::ptr::read_volatile(USART2_SR) & SR_TXE != 0 {
                break;
            }
        }
        core::ptr::write_volatile(USART2_DR, byte as u32);
    }
}

fn uart2_str(s: &str) {
    for b in s.bytes() {
        uart2_byte(b);
    }
}

fn uart2_u32(mut n: u32) {
    if n == 0 {
        uart2_byte(b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    for j in (0..i).rev() {
        uart2_byte(buf[j]);
    }
}

/// Enable GPIOA, TIM2, USART2, ADC1 clocks (all gated on APB/AHB out of reset).
fn clocks_init() {
    unsafe {
        core::ptr::write_volatile(
            RCC_AHB1ENR,
            core::ptr::read_volatile(RCC_AHB1ENR) | (1 << 0),
        );
        core::ptr::write_volatile(
            RCC_APB1ENR,
            core::ptr::read_volatile(RCC_APB1ENR) | (1 << 0) | (1 << 17),
        );
        core::ptr::write_volatile(
            RCC_APB2ENR,
            core::ptr::read_volatile(RCC_APB2ENR) | (1 << 8),
        );
    }
}

/// PA5 = general-purpose output (RC step drive). PA0 = analog (ADC1 CH0).
/// PA2 = AF7 (USART2_TX).
fn gpio_init() {
    unsafe {
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        let moder = (moder & !(0x3 << 10)) | (0x1 << 10); // PA5 output
        let moder = moder | (0x3 << 0); // PA0 analog
        let moder = (moder & !(0x3 << 4)) | (0x2 << 4); // PA2 alternate function
        core::ptr::write_volatile(GPIOA_MODER, moder);

        let afrl = core::ptr::read_volatile(GPIOA_AFRL);
        core::ptr::write_volatile(GPIOA_AFRL, (afrl & !(0xF << 8)) | (0x7 << 8));
        // PA2 -> AF7
    }
}

/// USART2 at 115200-8N1 off the 84 MHz core clock, APB1 undivided.
/// BRR = 84_000_000 / 115200 = 729.16 -> 729 = 0x2D9.
fn uart2_init() {
    unsafe {
        core::ptr::write_volatile(USART2_BRR, 0x2D9);
        core::ptr::write_volatile(USART2_CR1, (1 << 13) | (1 << 3)); // UE | TE
    }
}

/// Free-running microsecond tick: 84 MHz / 84 = 1 MHz, ARR at max so it
/// wraps every ~71.5 minutes (irrelevant at test-scale run lengths).
fn tim2_init() {
    unsafe {
        core::ptr::write_volatile(TIM2_PSC, 83);
        core::ptr::write_volatile(TIM2_ARR, 0xFFFF_FFFF);
        core::ptr::write_volatile(TIM2_EGR, 1); // UG: latch PSC, zero CNT
        core::ptr::write_volatile(TIM2_SR, 0);
        core::ptr::write_volatile(TIM2_CR1, 1); // CEN
    }
}

fn tim2_now() -> u32 {
    unsafe { core::ptr::read_volatile(TIM2_CNT) }
}

fn adc1_init() {
    unsafe {
        core::ptr::write_volatile(ADC1_CR2, 1); // ADON
    }
}

/// Trigger a single ADC1 conversion on channel 0 (PA0) and return the
/// 12-bit result.
fn adc1_read() -> u16 {
    unsafe {
        core::ptr::write_volatile(ADC1_CR2, 1 | (1 << 30)); // ADON + SWSTART
        let mut timeout = 100_000u32;
        loop {
            if core::ptr::read_volatile(ADC1_SR) & SR_EOC != 0 {
                break;
            }
            timeout -= 1;
            if timeout == 0 {
                return 0;
            }
        }
        (core::ptr::read_volatile(ADC1_DR) & 0xFFF) as u16
    }
}

fn led_pa5(on: bool) {
    unsafe {
        if on {
            core::ptr::write_volatile(GPIOA_BSRR, 1 << 5);
        } else {
            core::ptr::write_volatile(GPIOA_BSRR, 1 << (5 + 16));
        }
    }
}

/// 63.2 % of VDD: the voltage an RC node reaches one time constant after a
/// step. Crossing this level tells us tau directly.
const TAU_LEVEL_MV: u32 = (VDD_MV * 632) / 1000;
const TAU_MIN_US: u32 = 800;
const TAU_MAX_US: u32 = 1200;

fn sample_mv() -> (u16, u32) {
    let code = adc1_read();
    (code, (code as u32 * VDD_MV) / ADC_FULL_SCALE)
}

fn print_sample(t: u32, code: u16, mv: u32) {
    uart2_str("t=");
    uart2_u32(t);
    uart2_str(" adc=");
    uart2_u32(code as u32);
    uart2_str(" v=");
    uart2_u32(mv);
    uart2_str("\r\n");
}

/// State of the charge-curve check that runs after every PA5 rising edge.
struct RiseCheck {
    edge_us: u32,
    prev_t: u32,
    prev_mv: u32,
    monotonic: bool,
    active: bool,
}

impl RiseCheck {
    const fn idle() -> Self {
        RiseCheck {
            edge_us: 0,
            prev_t: 0,
            prev_mv: 0,
            monotonic: true,
            active: false,
        }
    }

    fn start(&mut self, edge_us: u32, mv_at_edge: u32) {
        self.edge_us = edge_us;
        self.prev_t = edge_us;
        self.prev_mv = mv_at_edge;
        self.monotonic = true;
        self.active = true;
    }

    /// Feed one sample. Prints `tau_us=<n>` and `rc_shape=ok|bad` the first
    /// time the reading crosses the 63.2 % level, then deactivates.
    fn feed(&mut self, t: u32, mv: u32) {
        if !self.active {
            return;
        }
        if mv < self.prev_mv {
            self.monotonic = false;
        }
        if mv >= TAU_LEVEL_MV {
            // Linear interpolation between the previous sample and this one.
            let dt = t.wrapping_sub(self.prev_t);
            let dv = mv - self.prev_mv;
            let cross_t = if dv == 0 {
                t
            } else {
                self.prev_t
                    .wrapping_add((TAU_LEVEL_MV.saturating_sub(self.prev_mv) * dt) / dv)
            };
            let tau = cross_t.wrapping_sub(self.edge_us);
            uart2_str("tau_us=");
            uart2_u32(tau);
            uart2_str("\r\n");
            let ok = self.monotonic && (TAU_MIN_US..=TAU_MAX_US).contains(&tau);
            uart2_str(if ok {
                "rc_shape=ok\r\n"
            } else {
                "rc_shape=bad\r\n"
            });
            self.active = false;
            return;
        }
        self.prev_t = t;
        self.prev_mv = mv;
    }
}

#[entry]
fn main() -> ! {
    clocks_init();
    gpio_init();
    uart2_init();
    tim2_init();
    adc1_init();

    uart2_str("RC Oscilloscope Lab\r\n");
    uart2_str("PA5 steps the RC net every 5ms, ADC1 CH0 (PA0) samples every 500us\r\n");

    let mut led_on = false;
    let mut next_toggle_us = TOGGLE_PERIOD_US;
    let mut next_sample_us = SAMPLE_PERIOD_US;
    let mut rise = RiseCheck::idle();
    led_pa5(led_on);

    loop {
        let now = tim2_now();

        if now.wrapping_sub(next_toggle_us) < (u32::MAX / 2) {
            led_on = !led_on;
            led_pa5(led_on);
            if led_on {
                // Reading at the edge anchors the interpolation.
                let (_, mv) = sample_mv();
                rise.start(now, mv);
            } else {
                rise.active = false;
            }
            next_toggle_us = next_toggle_us.wrapping_add(TOGGLE_PERIOD_US);
        }

        if now.wrapping_sub(next_sample_us) < (u32::MAX / 2) {
            let (code, mv) = sample_mv();
            print_sample(now, code, mv);
            rise.feed(now, mv);
            next_sample_us = next_sample_us.wrapping_add(SAMPLE_PERIOD_US);
        }
    }
}

// Silence unused-const warning: TICK_HZ documents the PSC derivation above
// but is not read back at runtime.
#[allow(dead_code)]
const _: u32 = TICK_HZ;
