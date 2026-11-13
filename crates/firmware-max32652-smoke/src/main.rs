#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// Renode platforms/cpus/max32652.repl instance bases; register offsets from
// MSDK MAX32650 uart_regs.h / gpio_regs.h.
const UART0: usize = 0x4004_2000;
const UART1: usize = 0x4004_3000;
const UART2: usize = 0x4004_4000;
const UART_STAT: usize = 0x08;
const UART_FIFO: usize = 0x1C;
const STAT_RX_EMPTY: u32 = 1 << 4;
const STAT_TX_EMPTY: u32 = 1 << 6;

const GPIO0: usize = 0x4000_8000;
const GPIO2: usize = 0x4000_A000;
const GPIO3: usize = 0x4000_B000;
const EN: usize = 0x00;
const EN_SET: usize = 0x04;
const EN_CLR: usize = 0x08;
const OUT_EN: usize = 0x0C;
const OUT_EN_SET: usize = 0x10;
const OUT_EN_CLR: usize = 0x14;
const OUT: usize = 0x18;
const OUT_SET: usize = 0x1C;
const OUT_CLR: usize = 0x20;
const IN: usize = 0x24;
const IN_EN: usize = 0x30;
const PDPU_SEL0: usize = 0x60;
const PDPU_SEL1: usize = 0x64;

// EvKit_V1 board.c: LED0 P2.25, button 0 P2.28.
const LED0: u32 = 1 << 25;
const PB0: u32 = 1 << 28;
// P1.7: no board I/O on it, so only the internal pull holds the pad.
const GPIO1: usize = 0x4000_9000;
const FREE: u32 = 1 << 7;

const SRAM_TOP_WORD: *mut u32 = 0x200F_FFFC as *mut u32;
// Smoke summary: 0xA3260000 | pass bits.
const SUMMARY: *mut u32 = 0x200F_F000 as *mut u32;

fn rd(addr: usize) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: usize, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(uart: usize, byte: u8) {
    let mut remaining = 100_000u32;
    while rd(uart + UART_STAT) & STAT_TX_EMPTY == 0 && remaining > 0 {
        remaining -= 1;
    }
    wr(uart + UART_FIFO, u32::from(byte));
}

fn puts(uart: usize, s: &[u8]) {
    for byte in s {
        putc(uart, *byte);
    }
}

#[entry]
fn main() -> ! {
    let mut pass = 0u32;
    let mut marks = [b'F'; 10];
    let mut check = |i: usize, ok: bool| {
        if ok {
            pass |= 1 << i;
            marks[i] = b'P';
        }
    };

    // 0-2: every UART STAT resets to RX_EMPTY | TX_EMPTY.
    for (i, uart) in [UART0, UART1, UART2].into_iter().enumerate() {
        check(i, rd(uart + UART_STAT) == STAT_RX_EMPTY | STAT_TX_EMPTY);
    }

    // 3: GPIO EN and IN_EN reset to all ones (every pad GPIO, input on).
    check(
        3,
        rd(GPIO0 + EN) == u32::MAX && rd(GPIO0 + IN_EN) == u32::MAX,
    );

    // 4: OUT_EN_SET / OUT_SET / OUT_CLR act on the latch; OUT_SET reads 0.
    wr(GPIO2 + OUT_EN_SET, LED0);
    wr(GPIO2 + OUT_SET, LED0 | (1 << 24));
    wr(GPIO2 + OUT_CLR, 1 << 24);
    check(
        4,
        rd(GPIO2 + OUT) == LED0 && rd(GPIO2 + OUT_SET) == 0 && rd(GPIO2 + OUT_EN) == LED0,
    );

    // 5: IN reads the driven pad; OUT_CLR drives it low again.
    let high = rd(GPIO2 + IN) & LED0;
    wr(GPIO2 + OUT_CLR, LED0);
    check(5, high == LED0 && rd(GPIO2 + IN) & LED0 == 0);

    // 6: an undriven input follows PDPU_SEL0 (up) / PDPU_SEL1 (down)
    // (MSDK MXC_GPIO_Config pad selections).
    wr(GPIO1 + OUT_EN_CLR, FREE);
    wr(GPIO1 + PDPU_SEL0, rd(GPIO1 + PDPU_SEL0) | FREE);
    let up = rd(GPIO1 + IN) & FREE;
    wr(GPIO1 + PDPU_SEL0, rd(GPIO1 + PDPU_SEL0) & !FREE);
    wr(GPIO1 + PDPU_SEL1, rd(GPIO1 + PDPU_SEL1) | FREE);
    let down = rd(GPIO1 + IN) & FREE;
    wr(GPIO1 + PDPU_SEL1, rd(GPIO1 + PDPU_SEL1) & !FREE);
    check(6, up == FREE && down == 0);

    // 7: EN_CLR gives a pad to its alternate function; EN_SET returns it.
    wr(GPIO2 + EN_CLR, PB0);
    let af = rd(GPIO2 + EN) & PB0;
    wr(GPIO2 + EN_SET, PB0);
    check(7, af == 0 && rd(GPIO2 + EN) == u32::MAX);

    // 8: port 3 at 0x4000_B000 answers (OUT latch round trip).
    wr(GPIO3 + OUT, 0x3FF);
    let p3 = rd(GPIO3 + OUT);
    wr(GPIO3 + OUT, 0);
    check(8, p3 == 0x3FF);

    // 9: the last word of the 1 MiB SRAM holds a value.
    unsafe { write_volatile(SRAM_TOP_WORD, 0x1F_FFFC) };
    check(9, unsafe { read_volatile(SRAM_TOP_WORD) } == 0x1F_FFFC);

    puts(UART0, b"MAX32652 UART0 smoke\r\n");
    // LED0: three pulses for the gpio_edges assertion.
    for _ in 0..3 {
        wr(GPIO2 + OUT_SET, LED0);
        wr(GPIO2 + OUT_CLR, LED0);
    }
    unsafe { write_volatile(SUMMARY, 0xA326_0000 | pass) };
    puts(UART1, b"MAX32652 UART1 line\r\n");
    puts(UART2, b"MAX32652 UART2 line\r\n");
    puts(UART0, b"MAX32652 checks ");
    puts(UART0, &marks);
    puts(UART0, b" DONE\r\n");
    loop {
        core::hint::spin_loop();
    }
}
