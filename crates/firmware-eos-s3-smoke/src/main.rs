#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// Renode platforms/cpus/eos-s3.repl: PL011 at 0x4001_0000.
const UART_DATA: *mut u32 = 0x4001_0000 as *mut u32;
// m4mem: 512 KiB SRAM at 0x2000_0000, mirrored at 0x0.
const SRAM: u32 = 0x2000_0000;
// Renode systemDmaSram: 4 KiB at 0x4000_F000.
const SDMA_SRAM: *mut u32 = 0x4000_F000 as *mut u32;
// qorc-sdk eoss3_dev.h: MISC_CTRL 0x4000_5000, IO_INPUT +0x100, IO_OUTPUT +0x104.
const IO_INPUT: *mut u32 = 0x4000_5100 as *mut u32;
const IO_OUTPUT: *mut u32 = 0x4000_5104 as *mut u32;
// Smoke summary: 0xE0530000 | pass bits.
const SUMMARY: *mut u32 = 0x2007_FF00 as *mut u32;

static MARKER: u32 = 0x5EED_E053;
static mut DATA_WORD: u32 = 0;

fn puts(s: &[u8]) {
    for byte in s {
        unsafe { write_volatile(UART_DATA, u32::from(*byte)) };
    }
}

#[entry]
fn main() -> ! {
    let mut pass = 0u32;
    let mut marks = [b'F'; 6];
    let mut check = |i: usize, ok: bool| {
        if ok {
            pass |= 1 << i;
            marks[i] = b'P';
        }
    };
    unsafe {
        // 0: code linked at 0x0 reads back through the 0x2000_0000 view.
        let marker = &MARKER as *const u32 as u32;
        check(
            0,
            read_volatile((SRAM + marker) as *const u32) == 0x5EED_E053,
        );

        // 1: a .data store at 0x2004_xxxx reads back through the 0x0 view.
        let data = core::ptr::addr_of_mut!(DATA_WORD);
        write_volatile(data, 0xA11A_5001);
        check(
            1,
            read_volatile((data as u32 - SRAM) as *const u32) == 0xA11A_5001,
        );

        // 2: a store through the 0x0 view lands in SRAM.
        let low = 0x0003_F000 as *mut u32;
        write_volatile(low, 0x0B5E_55ED);
        check(
            2,
            read_volatile((SRAM + 0x0003_F000) as *const u32) == 0x0B5E_55ED,
        );

        // 3: SDMA SRAM holds a word at both ends.
        write_volatile(SDMA_SRAM, 0x5D3A_0001);
        write_volatile(SDMA_SRAM.add(1023), 0x5D3A_03FF);
        check(
            3,
            read_volatile(SDMA_SRAM) == 0x5D3A_0001
                && read_volatile(SDMA_SRAM.add(1023)) == 0x5D3A_03FF,
        );

        // 4: IO_OUTPUT keeps bits [7:0] only.
        write_volatile(IO_OUTPUT, 0xFFFF_FF50);
        let out = read_volatile(IO_OUTPUT);
        write_volatile(IO_OUTPUT, 0);
        check(4, out == 0x50 && read_volatile(IO_OUTPUT) == 0);

        // 5: IO_INPUT reads the pad inputs (none driven), not IO_OUTPUT.
        write_volatile(IO_OUTPUT, 0xFF);
        let input = read_volatile(IO_INPUT);
        write_volatile(IO_OUTPUT, 0);
        check(5, input == 0);
    }

    puts(b"EOS S3 Cortex-M4F PL011 smoke\r\n");
    // GPIO 4: three pulses for the gpio_edges assertion.
    for _ in 0..3 {
        unsafe {
            write_volatile(IO_OUTPUT, read_volatile(IO_OUTPUT) | (1 << 4));
            write_volatile(IO_OUTPUT, read_volatile(IO_OUTPUT) & !(1 << 4));
        }
    }
    unsafe { write_volatile(SUMMARY, 0xE053_0000 | pass) };
    puts(b"EOS S3 checks ");
    puts(&marks);
    puts(b" DONE\r\n");
    loop {
        core::hint::spin_loop();
    }
}
