//! Cortex-M7 FPv5-DP architectural state smoke for the NUCLEO-F767ZI profile.

use core::arch::asm;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

use crate::f746_checks::{hex, puts, verdict};

const IRQ: u32 = 54;
const IRQ_BIT: u32 = 1 << (IRQ % 32);
const VECTOR_INDEX: usize = 16 + IRQ as usize;
const VECTOR_WORDS: usize = 256;

const SCB_CPACR: u32 = 0xE000_ED88;
const SCB_VTOR: u32 = 0xE000_ED08;
const NVIC_ISER1: u32 = 0xE000_E104;
const NVIC_ICER1: u32 = 0xE000_E184;
const NVIC_ISPR1: u32 = 0xE000_E204;
const NVIC_ICPR1: u32 = 0xE000_E284;
const NVIC_IABR1: u32 = 0xE000_E304;
const NVIC_IPR: u32 = 0xE000_E400;
const FPU_FPCCR: u32 = 0xE000_EF34;
const FPU_FPCAR: u32 = 0xE000_EF38;
const FPU_MVFR0: u32 = 0xE000_EF40;
const FPU_MVFR1: u32 = 0xE000_EF44;

const CPACR_FULL_ACCESS: u32 = 0x00F0_0000;
const FPCCR_ASPEN_LSPEN: u32 = 0xC000_0000;
const FPCCR_LSPACT: u32 = 1;
const CONTROL_FPCA: u32 = 1 << 2;
const THREAD_S0: u32 = 0x3F80_0000;
const HANDLER_S0: u32 = 0x4000_0000;

#[repr(C, align(1024))]
struct RamVectorTable([u32; VECTOR_WORDS]);

static mut RAM_VECTOR_TABLE: RamVectorTable = RamVectorTable([0; VECTOR_WORDS]);
static mut FPU_RECORD: [u32; 6] = [0; 6];

/// Check FPv5 double arithmetic and the lazy extended exception frame on M7.
pub fn run() {
    let mvfr_ok = unsafe { read_volatile(FPU_MVFR0 as *const u32) } == 0x1011_0221
        && unsafe { read_volatile(FPU_MVFR1 as *const u32) } == 0x1200_0011;

    let prior_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }

    let prior_cpacr = unsafe { read_volatile(SCB_CPACR as *const u32) };
    let prior_fpccr = unsafe { read_volatile(FPU_FPCCR as *const u32) };
    let prior_fpcar = unsafe { read_volatile(FPU_FPCAR as *const u32) };
    let prior_vtor = unsafe { read_volatile(SCB_VTOR as *const u32) };
    let irq_enabled = unsafe { read_volatile(NVIC_ISER1 as *const u32) } & IRQ_BIT != 0;
    let irq_pending = unsafe { read_volatile(NVIC_ISPR1 as *const u32) } & IRQ_BIT != 0;
    let irq_active = unsafe { read_volatile(NVIC_IABR1 as *const u32) } & IRQ_BIT != 0;
    let priority = unsafe { read_volatile((NVIC_IPR + IRQ) as *const u8) };
    let clean_irq_state = !irq_pending && !irq_active;

    unsafe {
        write_volatile(NVIC_ICER1 as *mut u32, IRQ_BIT);
        write_volatile(NVIC_ICPR1 as *mut u32, IRQ_BIT);
        write_volatile(SCB_CPACR as *mut u32, prior_cpacr | CPACR_FULL_ACCESS);
        write_volatile(FPU_FPCCR as *mut u32, FPCCR_ASPEN_LSPEN);
        write_volatile(FPU_FPCAR as *mut u32, 0);
        write_volatile((NVIC_IPR + IRQ) as *mut u8, 0x80);

        let vector_table = addr_of_mut!(RAM_VECTOR_TABLE.0).cast::<u32>();
        for index in 0..VECTOR_WORDS {
            write_volatile(vector_table.add(index), 0);
        }
        write_volatile(
            vector_table.add(VECTOR_INDEX),
            (fpu_irq_handler as *const () as usize as u32) | 1,
        );
        write_volatile(SCB_VTOR as *mut u32, vector_table as u32);
        for index in 0..6 {
            write_volatile(addr_of_mut!(FPU_RECORD).cast::<u32>().add(index), 0);
        }
        write_volatile(NVIC_ISER1 as *mut u32, IRQ_BIT);
        asm!("dsb", "isb", options(nostack, preserves_flags));
    }

    let double_add_ok = double_precision_addition();
    unsafe {
        asm!(
            "vmov s0, {thread_s0}",
            thread_s0 = in(reg) THREAD_S0,
            out("s0") _,
            options(nostack),
        );
    }
    let msp_before: u32;
    unsafe {
        asm!("mrs {msp}, MSP", msp = out(reg) msp_before, options(nomem, nostack));
        asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
        write_volatile(NVIC_ISPR1 as *mut u32, IRQ_BIT);
        asm!("dsb", "isb", options(nostack, preserves_flags));
    }

    let record = addr_of!(FPU_RECORD).cast::<u32>();
    let read_record = |index: usize| unsafe { read_volatile(record.add(index)) };
    let frame_sp = read_record(0);
    let exc_return = read_record(1);
    let fpccr_entry = read_record(2);
    let fpcar_entry = read_record(3);
    let fpccr_after_touch = read_record(4);
    let saved_s0 = read_record(5);
    let fpcar_after = unsafe { read_volatile(FPU_FPCAR as *const u32) };
    let pending_after = unsafe { read_volatile(NVIC_ISPR1 as *const u32) } & IRQ_BIT != 0;
    let active_after = unsafe { read_volatile(NVIC_IABR1 as *const u32) } & IRQ_BIT != 0;
    let msp_after: u32;
    let restored_s0: u32;
    let control_after: u32;
    unsafe {
        asm!(
            "mrs {msp}, MSP",
            "vmov {s0}, s0",
            "mrs {control}, CONTROL",
            msp = out(reg) msp_after,
            s0 = out(reg) restored_s0,
            control = out(reg) control_after,
            options(nomem, nostack),
        );
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }

    let lazy_frame = fpccr_entry & FPCCR_LSPACT != 0
        && fpccr_after_touch & FPCCR_LSPACT == 0
        && fpcar_entry == frame_sp.wrapping_add(0x20)
        && fpcar_after == fpcar_entry
        && saved_s0 == THREAD_S0;
    let context_restored = restored_s0 == THREAD_S0 && control_after & CONTROL_FPCA != 0;
    let passed = mvfr_ok
        && double_add_ok
        && prior_primask == 0
        && clean_irq_state
        && exc_return == 0xFFFF_FFE9
        && frame_sp == msp_before.wrapping_sub(0x68)
        && lazy_frame
        && context_restored
        && !pending_after
        && !active_after
        && msp_after == msp_before;

    unsafe {
        write_volatile(NVIC_ICER1 as *mut u32, IRQ_BIT);
        write_volatile(NVIC_ICPR1 as *mut u32, IRQ_BIT);
        write_volatile(SCB_VTOR as *mut u32, prior_vtor);
        write_volatile((NVIC_IPR + IRQ) as *mut u8, priority);
        if irq_enabled {
            write_volatile(NVIC_ISER1 as *mut u32, IRQ_BIT);
        }
        if irq_pending {
            write_volatile(NVIC_ISPR1 as *mut u32, IRQ_BIT);
        }
        write_volatile(FPU_FPCAR as *mut u32, prior_fpcar);
        write_volatile(FPU_FPCCR as *mut u32, prior_fpccr);
        write_volatile(SCB_CPACR as *mut u32, prior_cpacr);
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    puts("fpu m7 dp=");
    puts(if double_add_ok { "1" } else { "0" });
    puts(" mvfr=");
    puts(if mvfr_ok { "1" } else { "0" });
    puts(" exc_return=");
    hex(exc_return);
    puts(" lazy=");
    puts(if fpccr_entry & FPCCR_LSPACT != 0 {
        "1->"
    } else {
        "0->"
    });
    puts(if fpccr_after_touch & FPCCR_LSPACT == 0 {
        "0"
    } else {
        "1"
    });
    puts(" fpcar=");
    puts(
        if fpcar_entry == frame_sp.wrapping_add(0x20) && fpcar_after == fpcar_entry {
            "1"
        } else {
            "0"
        },
    );
    puts(" saved=");
    puts(if saved_s0 == THREAD_S0 { "1" } else { "0" });
    puts(" restored=");
    puts(if context_restored { "1" } else { "0" });
    puts(" ");
    verdict(passed);
}

fn double_precision_addition() -> bool {
    let mut low = 0;
    let mut high = 0x3FF0_0000;
    unsafe {
        asm!(
            "vmov d0, {low}, {high}",
            "vadd.f64 d0, d0, d0",
            "vmov {low}, {high}, d0",
            low = inout(reg) low,
            high = inout(reg) high,
            out("d0") _,
            options(nostack),
        );
    }
    low == 0 && high == 0x4000_0000
}

#[inline(never)]
unsafe fn clobber_s0_in_handler() {
    asm!(
        "vmov s0, {value}",
        value = in(reg) HANDLER_S0,
        out("s0") _,
        options(nostack),
    );
}

#[unsafe(naked)]
unsafe extern "C" fn fpu_irq_handler() {
    core::arch::naked_asm!(
        "mov r2, lr",
        "mrs r0, msp",
        "ldr r1, ={record}",
        "str r0, [r1, #0]",
        "str r2, [r1, #4]",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r1, #8]",
        "ldr r3, ={fpcar}",
        "ldr r3, [r3]",
        "str r3, [r1, #12]",
        "push {{r4-r7}}",
        "mov r4, r0",
        "mov r5, r1",
        "bl {touch}",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r5, #16]",
        "ldr r3, [r4, #32]",
        "str r3, [r5, #20]",
        "pop {{r4-r7}}",
        "ldr r2, ={record}",
        "ldr r2, [r2, #4]",
        "mov lr, r2",
        "bx lr",
        record = sym FPU_RECORD,
        fpccr = const FPU_FPCCR,
        fpcar = const FPU_FPCAR,
        touch = sym clobber_s0_in_handler,
    );
}
