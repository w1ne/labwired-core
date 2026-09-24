// LabWired - Firmware Simulation Platform
// Real `bkpt #0xAB` semihosting smoke. No mocked syscall: r0/r1 are set and
// the trap is the Thumb breakpoint itself.
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

static MSG: &[u8] = b"semihost hello\n\0";

fn semihost(op: u32, arg: u32) {
    unsafe {
        core::arch::asm!(
            "bkpt #0xAB",
            in("r0") op,
            in("r1") arg,
            options(preserves_flags, nostack),
        );
    }
}

#[entry]
fn main() -> ! {
    // SYS_WRITE0: r1 points at the NUL-terminated string. The NUL is not written.
    semihost(0x04, MSG.as_ptr() as u32);
    // SYS_EXIT: r1 is the reason itself, not a pointer. 0x20026 is application exit.
    semihost(0x18, 0x20026);
    // Unreachable when SYS_EXIT stops the machine. Keeps the binary from
    // falling off the end if the trap is ignored.
    loop {
        core::hint::spin_loop();
    }
}
