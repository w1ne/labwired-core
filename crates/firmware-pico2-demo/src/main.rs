#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2350 register addresses, from its address map (not the RP2040 map).
const UART0_DR: *mut u32 = 0x4007_0000 as *mut u32;
const GPIO25_CTRL: *mut u32 = 0x4002_80CC as *mut u32;
const SIO_GPIO_OUT_SET: *mut u32 = 0xD000_0014 as *mut u32;
const SIO_GPIO_OUT_CLR: *mut u32 = 0xD000_0018 as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = 0xD000_0024 as *mut u32;
const PIO0_BASE: usize = 0x5020_0000;

const GPIO_FUNC_SIO: u32 = 5;
const GPIO_FUNC_PIO0: u32 = 6;
const LED_PIN: u32 = 25;

#[entry]
fn main() -> ! {
    unsafe {
        // Route GP25 through SIO, then enable it as an output. Pico 2's user
        // LED is attached to this pad in the board system profile.
        core::ptr::write_volatile(GPIO25_CTRL, GPIO_FUNC_SIO);
        core::ptr::write_volatile(SIO_GPIO_OE_SET, 1 << LED_PIN);
    }

    // The UART assertion observes the modeled PL011 data-register path. USB
    // CDC and the physical UART waveform are not part of this smoke.
    print_uart("PICO2_SMOKE_OK\n");

    loop {
        for _ in 0..4 {
            unsafe { core::ptr::write_volatile(SIO_GPIO_OUT_SET, 1 << LED_PIN) };
            delay(1_000);
            unsafe { core::ptr::write_volatile(SIO_GPIO_OUT_CLR, 1 << LED_PIN) };
            delay(1_000);
        }

        // Switch GP25 to PIO0 and run the LED pin through a state machine.
        // The program sets the output direction, alternates the pin, and jumps
        // back to the high transition.
        unsafe {
            core::ptr::write_volatile(GPIO25_CTRL, GPIO_FUNC_PIO0);

            let pio0 = PIO0_BASE as *mut u32;
            let sm0_exec_ctrl = (PIO0_BASE + 0x0CC) as *mut u32;
            let sm0_pin_ctrl = (PIO0_BASE + 0x0DC) as *mut u32;

            core::ptr::write_volatile(sm0_pin_ctrl, (1 << 26) | (LED_PIN << 5));
            core::ptr::write_volatile(pio0.add(0x48 / 4), 0xE081); // SET PINDIRS, 1
            core::ptr::write_volatile(pio0.add(0x4C / 4), 0xE001); // SET PINS, 1
            core::ptr::write_volatile(pio0.add(0x50 / 4), 0xE000); // SET PINS, 0
            core::ptr::write_volatile(pio0.add(0x54 / 4), 0x0001); // JMP 1
            core::ptr::write_volatile(sm0_exec_ctrl, 3 << 12); // wrap at instruction 3
            core::ptr::write_volatile(pio0, 1); // enable SM0
        }

        // Let PIO0 produce transitions before reporting its marker. The smoke
        // independently asserts the GP25 edge count as well as this UART line.
        delay(64);
        print_uart("PICO2_PIO_OK\n");
        loop {
            cortex_m::asm::nop();
        }
    }
}

fn print_uart(s: &str) {
    for byte in s.bytes() {
        unsafe { core::ptr::write_volatile(UART0_DR, byte as u32) };
    }
}

fn delay(n: u32) {
    for _ in 0..n {
        cortex_m::asm::nop();
    }
}
