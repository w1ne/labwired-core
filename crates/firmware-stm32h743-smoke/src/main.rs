#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB4ENR: *mut u32 = 0x5802_44E0 as *mut u32;
const RCC_APB1LENR: *mut u32 = 0x5802_44E8 as *mut u32;
const GPIOB: u32 = 0x5802_0400;
const GPIOD: u32 = 0x5802_0C00;
const USART3: u32 = 0x4000_4800;
const MODER: u32 = 0x00;
const AFRH: u32 = 0x24;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;
const USART_CR1: *mut u32 = USART3 as *mut u32;
const USART_BRR: *mut u32 = (USART3 + 0x0C) as *mut u32;
const USART_ISR: *const u32 = (USART3 + 0x1C) as *const u32;
const USART_RDR: *const u32 = (USART3 + 0x24) as *const u32;
const USART_TDR: *mut u32 = (USART3 + 0x28) as *mut u32;
const USART_RXNEIE: u32 = 1 << 5;
const USART_TXEIE: u32 = 1 << 7;
const USART_TCIE: u32 = 1 << 6;
const NVIC_ISPR1: *const u32 = 0xE000_E204 as *const u32;
const NVIC_ICPR1: *mut u32 = 0xE000_E284 as *mut u32;
const USART3_IRQ_BIT: u32 = 1 << (39 - 32);
const TXE: u32 = 1 << 7;
const RXNE: u32 = 1 << 5;
const TC: u32 = 1 << 6;
const USART_RECEIVER_ENABLE: u32 = if cfg!(feature = "nucleo-h753zi-rx") {
    1 << 2
} else {
    0
};
const LED: u32 = 1;

fn register_rmw(address: u32, clear: u32, set: u32) {
    unsafe {
        let register = address as *mut u32;
        write_volatile(register, (read_volatile(register) & !clear) | set);
    }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART_ISR) & TXE == 0 {}
        write_volatile(USART_TDR, byte as u32);
    }
}

fn send(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

#[cfg(feature = "nucleo-h753zi-rx")]
fn check_uart_rx() {
    let mut first = 0;
    let mut first_ready = false;
    for _ in 0..1_000_000 {
        if unsafe { read_volatile(USART_ISR) } & RXNE != 0 {
            first_ready = true;
            break;
        }
    }

    // USART3_IRQn is 39 on STM32H753; observe its pending bit in NVIC ISPR1.
    // The imported profile wires this line only for the exact NUCLEO-H753ZI.
    unsafe { write_volatile(NVIC_ICPR1, USART3_IRQ_BIT) };
    let rxne_irq_isolated =
        first_ready && unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT == 0;
    unsafe {
        let cr1 = read_volatile(USART_CR1);
        write_volatile(USART_CR1, cr1 | USART_RXNEIE);
    }
    let mut rxne_irq_asserted = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT != 0 {
            rxne_irq_asserted = true;
            break;
        }
    }

    if first_ready {
        first = unsafe { read_volatile(USART_RDR) } & 0xFF;
    }

    // The injected stream has a second byte queued. Clearing NVIC pending
    // while RXNE remains set must let the USART level re-pend the same line.
    let second_still_pending = first_ready && unsafe { read_volatile(USART_ISR) } & RXNE != 0;
    unsafe { write_volatile(NVIC_ICPR1, USART3_IRQ_BIT) };
    let mut second_irq_reasserted = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT != 0 {
            second_irq_reasserted = true;
            break;
        }
    }

    let mut second = 0;
    let mut second_ready = false;
    for _ in 0..1_000_000 {
        if unsafe { read_volatile(USART_ISR) } & RXNE != 0 {
            second = unsafe { read_volatile(USART_RDR) } & 0xFF;
            second_ready = true;
            break;
        }
    }

    let cleared = unsafe { read_volatile(USART_ISR) } & RXNE == 0;
    unsafe { write_volatile(NVIC_ICPR1, USART3_IRQ_BIT) };
    let mut rxne_irq_released = true;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT != 0 {
            rxne_irq_released = false;
            break;
        }
    }

    if first_ready
        && first == b'R' as u32
        && second_ready
        && second == b'S' as u32
        && cleared
        && rxne_irq_isolated
        && rxne_irq_asserted
        && second_still_pending
        && second_irq_reasserted
        && rxne_irq_released
    {
        send(b"usart3 rx=00000052,00000053 rxne_irq=isolated,asserted,reasserted,released ok\r\n");
    } else {
        send(b"usart3 rx=fail rxne_irq=fail fail\r\n");
    }
}

#[cfg(feature = "nucleo-h753zi-usart3-irq")]
fn check_uart_txe_irq() {
    let mut txe_ready = false;
    for _ in 0..100_000 {
        if unsafe { read_volatile(USART_ISR) } & TXE != 0 {
            txe_ready = true;
            break;
        }
    }

    // USART3_IRQn is 39 on STM32H753; the imported profile wires this line.
    unsafe { write_volatile(NVIC_ICPR1, USART3_IRQ_BIT) };
    let txe_irq_isolated = txe_ready && unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT == 0;
    let cr1 = unsafe { read_volatile(USART_CR1) };
    unsafe { write_volatile(USART_CR1, cr1 | USART_TXEIE) };
    let mut txe_irq_asserted = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT != 0 {
            txe_irq_asserted = true;
            break;
        }
    }

    unsafe {
        write_volatile(USART_CR1, cr1 & !USART_TXEIE);
        write_volatile(NVIC_ICPR1, USART3_IRQ_BIT);
    }
    let mut txe_irq_released = true;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT != 0 {
            txe_irq_released = false;
            break;
        }
    }

    if txe_irq_isolated && txe_irq_asserted && txe_irq_released {
        send(b"usart3 txe_irq=isolated,asserted,released ok\r\n");
    } else {
        send(b"usart3 txe_irq=fail fail\r\n");
    }
}

#[cfg(feature = "nucleo-h753zi-usart3-irq")]
fn check_uart_tc_irq() {
    let mut tc_ready = false;
    for _ in 0..100_000 {
        if unsafe { read_volatile(USART_ISR) } & TC != 0 {
            tc_ready = true;
            break;
        }
    }

    // Keep TXEIE off so this probe isolates the imported model's TC interrupt.
    let cr1 = unsafe { read_volatile(USART_CR1) } & !(USART_TXEIE | USART_TCIE);
    unsafe {
        write_volatile(USART_CR1, cr1);
        write_volatile(NVIC_ICPR1, USART3_IRQ_BIT);
    }
    let tc_irq_isolated = tc_ready && unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT == 0;

    unsafe { write_volatile(USART_CR1, cr1 | USART_TCIE) };
    let mut tc_irq_asserted = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT != 0 {
            tc_irq_asserted = true;
            break;
        }
    }

    unsafe {
        write_volatile(USART_CR1, cr1);
        write_volatile(NVIC_ICPR1, USART3_IRQ_BIT);
    }
    let mut tc_irq_released = true;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT != 0 {
            tc_irq_released = false;
            break;
        }
    }

    if tc_irq_isolated && tc_irq_asserted && tc_irq_released {
        send(b"usart3 tc_irq=isolated,asserted,released ok\r\n");
    } else {
        send(b"usart3 tc_irq=fail fail\r\n");
    }
}

fn set_led(on: bool) {
    let bit = if on { LED } else { LED << 16 };
    unsafe { write_volatile((GPIOB + BSRR) as *mut u32, bit) };
}

fn led_is_on() -> bool {
    unsafe { read_volatile((GPIOB + ODR) as *const u32) & LED != 0 }
}

fn delay() {
    for _ in 0..200_000 {
        unsafe {
            let _ = read_volatile((GPIOB + ODR) as *const u32);
        }
    }
}

#[entry]
fn main() -> ! {
    // Enable GPIOB/GPIOD and USART3 in the STM32H7 RCC.
    unsafe {
        write_volatile(
            RCC_AHB4ENR,
            read_volatile(RCC_AHB4ENR) | (1 << 1) | (1 << 3),
        );
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 18));
    }

    // Route PD8 to USART3_TX (AF7), configure USART3, and make PB0 an output.
    register_rmw(GPIOD + MODER, 0b11 << 16, 0b10 << 16);
    register_rmw(GPIOD + AFRH, 0xF << 0, 7);
    register_rmw(GPIOB + MODER, 0b11, 0b01);
    unsafe {
        write_volatile(USART_BRR, 555); // 64 MHz / 115200
        write_volatile(USART_CR1, (1 << 0) | (1 << 3) | USART_RECEIVER_ENABLE); // UE | TE | optional RE
    }

    set_led(false);
    send(b"STM32H7 register smoke\r\n");
    #[cfg(feature = "nucleo-h753zi-rx")]
    check_uart_rx();
    #[cfg(feature = "nucleo-h753zi-usart3-irq")]
    check_uart_txe_irq();
    #[cfg(feature = "nucleo-h753zi-usart3-irq")]
    check_uart_tc_irq();
    loop {
        let on = !led_is_on();
        set_led(on);
        send(if on {
            b"gpiob0=high\r\n"
        } else {
            b"gpiob0=low\r\n"
        });
        delay();
    }
}
