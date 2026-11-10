#![no_std]
#![no_main]

use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

const RESULT: *mut u32 = 0x2000_0008 as *mut u32;
const PASS: *mut u32 = 0x2000_000C as *mut u32;
const EXCEPTION_CONTEXT: *mut u32 = 0x2000_0010 as *mut u32;
const EXCEPTION_FRAME_R0: *mut u32 = 0x2000_0014 as *mut u32;
const EXCEPTION_FRAME_PC: *mut u32 = 0x2000_0018 as *mut u32;
const EXCEPTION_FRAME_XPSR: *mut u32 = 0x2000_001C as *mut u32;
const EXCEPTION_FRAME_SP: *mut u32 = 0x2000_0020 as *mut u32;
const PADDED_EXCEPTION_CONTEXT: *mut u32 = 0x2000_0024 as *mut u32;
const PADDED_EXCEPTION_FRAME_XPSR: *mut u32 = 0x2000_0028 as *mut u32;
const PADDED_EXCEPTION_FRAME_SP: *mut u32 = 0x2000_002C as *mut u32;
const PADDED_EXCEPTION_PSP_AFTER: *mut u32 = 0x2000_0030 as *mut u32;
const UNALIGNED_EXCEPTION_CONTEXT: *mut u32 = 0x2000_0034 as *mut u32;
const UNALIGNED_EXCEPTION_FRAME_XPSR: *mut u32 = 0x2000_0038 as *mut u32;
const UNALIGNED_EXCEPTION_FRAME_SP: *mut u32 = 0x2000_003C as *mut u32;
const UNALIGNED_EXCEPTION_PSP_AFTER: *mut u32 = 0x2000_0040 as *mut u32;
const MSP_EXCEPTION_CONTEXT: *mut u32 = 0x2000_0044 as *mut u32;
const MSP_EXCEPTION_FRAME_R0: *mut u32 = 0x2000_0048 as *mut u32;
const MSP_EXCEPTION_FRAME_XPSR: *mut u32 = 0x2000_004C as *mut u32;
const MSP_EXCEPTION_FRAME_SP: *mut u32 = 0x2000_0050 as *mut u32;
const MSP_EXCEPTION_AFTER: *mut u32 = 0x2000_0054 as *mut u32;
const MSP_EXCEPTION_RESTORED: *mut u32 = 0x2000_0058 as *mut u32;
const MSP_EXCEPTION_PADDING: *mut u32 = 0x2000_005C as *mut u32;

const FLEXCOMM0: usize = 0x4011_0000;
const SCB_CCR: usize = 0xE000_ED14;
const SCB_CCR_STKALIGN: u32 = 1 << 9;
const NVIC_ISER0: usize = 0xE000_E100;
const NVIC_ICER0: usize = 0xE000_E180;
const NVIC_ISPR0: usize = 0xE000_E200;
const NVIC_ICPR0: usize = 0xE000_E280;
const FLEXCOMM0_IRQ_BIT: u32 = 1 << 7;
const FLEXCOMM0_IRQN: i16 = 7;
const FLEXCOMM0_EXCEPTION_NUMBER: u32 = 16 + FLEXCOMM0_IRQN as u32;
const FLEXCOMM0_HANDLER_MARK: u32 =
    (FLEXCOMM0_EXCEPTION_NUMBER << 16) | (FLEXCOMM0_IRQN as u32 + 1);
const LPUART_VERID: usize = 0x00;
const LPUART_PARAM: usize = 0x04;
const LPUART_BAUD: usize = 0x10;
const LPUART_STAT: usize = 0x14;
const LPUART_CTRL: usize = 0x18;
const LPUART_DATA: usize = 0x1C;
const LPUART_FIFO: usize = 0x28;
const LPUART_WATER: usize = 0x2C;
const LPFC_ISTAT: usize = 0xFF4;
const LPFC_PSELID: usize = 0xFF8;
const PSELID_ID_PRESENT: u32 = 0x0010_3070;
const PSELID_UART: u32 = 1;
const PSELID_LOCK: u32 = 1 << 3;
const STAT_TDRE: u32 = 1 << 23;
const STAT_TC: u32 = 1 << 22;
const STAT_RDRF: u32 = 1 << 21;
const CTRL_TIE_TE: u32 = (1 << 23) | (1 << 19);
const CTRL_TE: u32 = 1 << 19;
const CTRL_RE: u32 = 1 << 18;
const CTRL_RIE: u32 = 1 << 21;
const FIFO_RXFE: u32 = 1 << 3;
const WATER_RXCOUNT_SHIFT: u32 = 24;
const ISTAT_UARTRX: u32 = 1 << 1;

static IRQ_ENTRY: AtomicU32 = AtomicU32::new(0);
static IRQ_EXC_RETURN: AtomicU32 = AtomicU32::new(0);
static IRQ_STACKED_R0: AtomicU32 = AtomicU32::new(0);
static IRQ_STACKED_PC: AtomicU32 = AtomicU32::new(0);
static IRQ_STACKED_XPSR: AtomicU32 = AtomicU32::new(0);
static IRQ_STACKED_SP: AtomicU32 = AtomicU32::new(0);
static IRQ_CASE: AtomicU32 = AtomicU32::new(0);
static IRQ_PADDED_R0: AtomicU32 = AtomicU32::new(0);
static IRQ_PADDED_PC: AtomicU32 = AtomicU32::new(0);
static IRQ_PADDED_XPSR: AtomicU32 = AtomicU32::new(0);
static IRQ_PADDED_SP: AtomicU32 = AtomicU32::new(0);
static IRQ_UNALIGNED_R0: AtomicU32 = AtomicU32::new(0);
static IRQ_UNALIGNED_PC: AtomicU32 = AtomicU32::new(0);
static IRQ_UNALIGNED_XPSR: AtomicU32 = AtomicU32::new(0);
static IRQ_UNALIGNED_SP: AtomicU32 = AtomicU32::new(0);

const PSP_STACK_WORDS: usize = 64;
static mut PSP_STACK: [u32; PSP_STACK_WORDS] = [0; PSP_STACK_WORDS];

fn rd(off: usize) -> u32 {
    unsafe { read_volatile((FLEXCOMM0 + off) as *const u32) }
}

fn wr(off: usize, value: u32) {
    unsafe { write_volatile((FLEXCOMM0 + off) as *mut u32, value) }
}

fn nvic_irq_pending() -> bool {
    unsafe { read_volatile(NVIC_ISPR0 as *const u32) & FLEXCOMM0_IRQ_BIT != 0 }
}

fn clear_nvic_irq_pending() {
    unsafe { write_volatile(NVIC_ICPR0 as *mut u32, FLEXCOMM0_IRQ_BIT) }
}

/// Take the already-pending FLEXCOMM0 interrupt from Thread mode using PSP.
/// The handler records EXC_RETURN; after exception return this checks that PSP
/// popped the frame and MSP was restored after running the handler.
fn exercise_irq_exception_on_psp() -> bool {
    IRQ_CASE.store(1, Ordering::Relaxed);
    IRQ_ENTRY.store(0, Ordering::Relaxed);
    IRQ_EXC_RETURN.store(0, Ordering::Relaxed);
    IRQ_STACKED_R0.store(0, Ordering::Relaxed);
    IRQ_STACKED_PC.store(0, Ordering::Relaxed);
    IRQ_STACKED_XPSR.store(0, Ordering::Relaxed);
    IRQ_STACKED_SP.store(0, Ordering::Relaxed);
    let psp_top = addr_of_mut!(PSP_STACK) as u32 + (PSP_STACK_WORDS as u32) * 4;
    let msp_before: u32;
    let psp_after: u32;
    let msp_after: u32;

    unsafe {
        core::arch::asm!(
            "msr psp, {top}",
            "mrs {msp_before}, msp",
            "movs {tmp}, #2",
            "msr control, {tmp}",
            "isb",
            // Make the basic exception frame's R0 word independently
            // identifiable after the Cortex-M stacks Thread mode on PSP.
            "movw r0, #0x5678",
            "movt r0, #0x1234",
            "str {irq_bit}, [{iser}]",
            "movw {count}, #10000",
            "1:",
            "ldr {seen}, [{entry}]",
            "cmp {seen}, {mark}",
            "beq 2f",
            "subs {count}, #1",
            "bne 1b",
            "2:",
            "mrs {psp_after}, psp",
            "movs {tmp}, #0",
            "msr control, {tmp}",
            "isb",
            "mrs {msp_after}, msp",
            top = in(reg) psp_top,
            msp_before = out(reg) msp_before,
            psp_after = out(reg) psp_after,
            msp_after = out(reg) msp_after,
            tmp = out(reg) _,
            irq_bit = in(reg) FLEXCOMM0_IRQ_BIT,
            iser = in(reg) NVIC_ISER0 as u32,
            count = out(reg) _,
            seen = out(reg) _,
            entry = in(reg) addr_of!(IRQ_ENTRY) as u32,
            mark = in(reg) FLEXCOMM0_HANDLER_MARK,
            out("r0") _,
            options(nostack),
        );
    }

    let exc_return = IRQ_EXC_RETURN.load(Ordering::Relaxed);
    let handler_entered = IRQ_ENTRY.load(Ordering::Relaxed) == FLEXCOMM0_HANDLER_MARK;
    let frame_r0 = IRQ_STACKED_R0.load(Ordering::Relaxed);
    let frame_pc = IRQ_STACKED_PC.load(Ordering::Relaxed);
    let frame_xpsr = IRQ_STACKED_XPSR.load(Ordering::Relaxed);
    let frame_sp = IRQ_STACKED_SP.load(Ordering::Relaxed);
    let frame_ok = frame_r0 == 0x1234_5678
        && frame_pc & 1 == 0
        && (0x3800_0000..0x4000_0000).contains(&frame_pc)
        && frame_xpsr & (1 << 24) != 0
        && frame_xpsr & 0x1FF == 0
        && frame_xpsr & (1 << 9) == 0
        && frame_sp == psp_top - 8 * 4;
    let context = u32::from(exc_return == 0xFFFF_FFFD)
        | (u32::from(psp_after == psp_top) << 1)
        | (u32::from(msp_before == msp_after) << 2)
        | (u32::from(frame_ok) << 3);
    unsafe {
        write_volatile(EXCEPTION_CONTEXT, context);
        write_volatile(EXCEPTION_FRAME_R0, frame_r0);
        write_volatile(EXCEPTION_FRAME_PC, frame_pc);
        write_volatile(EXCEPTION_FRAME_XPSR, frame_xpsr);
        write_volatile(EXCEPTION_FRAME_SP, frame_sp);
    }
    handler_entered && context == 0b1111
}

/// Deliver a second FLEXCOMM0 interrupt from a four-byte-aligned PSP. The
/// Cortex-M exception frame should include its alignment word and set xPSR[9].
fn exercise_irq_exception_on_misaligned_psp() -> bool {
    IRQ_CASE.store(2, Ordering::Relaxed);
    IRQ_ENTRY.store(0, Ordering::Relaxed);
    IRQ_EXC_RETURN.store(0, Ordering::Relaxed);
    IRQ_PADDED_R0.store(0, Ordering::Relaxed);
    IRQ_PADDED_PC.store(0, Ordering::Relaxed);
    IRQ_PADDED_XPSR.store(0, Ordering::Relaxed);
    IRQ_PADDED_SP.store(0, Ordering::Relaxed);

    let psp_top = addr_of_mut!(PSP_STACK) as u32 + (PSP_STACK_WORDS as u32) * 4;
    let psp_before = psp_top - 4;
    let msp_before: u32;
    let psp_after: u32;
    let msp_after: u32;

    unsafe {
        core::arch::asm!(
            "msr psp, {top}",
            "mrs {msp_before}, msp",
            "movs {tmp}, #2",
            "msr control, {tmp}",
            "isb",
            // The second exception gets a distinct stacked value and an SP
            // whose bit 2 is set, requiring the optional alignment word.
            "movw r0, #0x4321",
            "movt r0, #0x8765",
            "str {irq_bit}, [{iser}]",
            "movw {count}, #10000",
            "1:",
            "ldr {seen}, [{entry}]",
            "cmp {seen}, {mark}",
            "beq 2f",
            "subs {count}, #1",
            "bne 1b",
            "2:",
            "mrs {psp_after}, psp",
            "movs {tmp}, #0",
            "msr control, {tmp}",
            "isb",
            "mrs {msp_after}, msp",
            top = in(reg) psp_before,
            msp_before = out(reg) msp_before,
            psp_after = out(reg) psp_after,
            msp_after = out(reg) msp_after,
            tmp = out(reg) _,
            irq_bit = in(reg) FLEXCOMM0_IRQ_BIT,
            iser = in(reg) NVIC_ISER0 as u32,
            count = out(reg) _,
            seen = out(reg) _,
            entry = in(reg) addr_of!(IRQ_ENTRY) as u32,
            mark = in(reg) FLEXCOMM0_HANDLER_MARK,
            out("r0") _,
            options(nostack),
        );
    }

    let exc_return = IRQ_EXC_RETURN.load(Ordering::Relaxed);
    let handler_entered = IRQ_ENTRY.load(Ordering::Relaxed) == FLEXCOMM0_HANDLER_MARK;
    let frame_r0 = IRQ_PADDED_R0.load(Ordering::Relaxed);
    let frame_pc = IRQ_PADDED_PC.load(Ordering::Relaxed);
    let frame_xpsr = IRQ_PADDED_XPSR.load(Ordering::Relaxed);
    let frame_sp = IRQ_PADDED_SP.load(Ordering::Relaxed);
    let frame_ok = frame_r0 == 0x8765_4321
        && frame_pc & 1 == 0
        && (0x3800_0000..0x4000_0000).contains(&frame_pc)
        && frame_xpsr & (1 << 24) != 0
        && frame_xpsr & 0x1FF == 0
        && frame_xpsr & (1 << 9) != 0
        && frame_sp == psp_top - 10 * 4;
    let context = u32::from(exc_return == 0xFFFF_FFFD)
        | (u32::from(psp_after == psp_before) << 1)
        | (u32::from(msp_before == msp_after) << 2)
        | (u32::from(frame_ok) << 3);
    unsafe {
        write_volatile(PADDED_EXCEPTION_CONTEXT, context);
        write_volatile(PADDED_EXCEPTION_FRAME_XPSR, frame_xpsr);
        write_volatile(PADDED_EXCEPTION_FRAME_SP, frame_sp);
        write_volatile(PADDED_EXCEPTION_PSP_AFTER, psp_after);
    }
    handler_entered && context == 0b1111
}

/// Clear CCR.STKALIGN and confirm that a four-byte-aligned PSP is preserved
/// without the optional exception-frame word or stacked xPSR marker.
fn exercise_irq_exception_without_stack_alignment() -> bool {
    IRQ_CASE.store(3, Ordering::Relaxed);
    IRQ_ENTRY.store(0, Ordering::Relaxed);
    IRQ_EXC_RETURN.store(0, Ordering::Relaxed);
    IRQ_UNALIGNED_R0.store(0, Ordering::Relaxed);
    IRQ_UNALIGNED_PC.store(0, Ordering::Relaxed);
    IRQ_UNALIGNED_XPSR.store(0, Ordering::Relaxed);
    IRQ_UNALIGNED_SP.store(0, Ordering::Relaxed);

    let psp_top = addr_of_mut!(PSP_STACK) as u32 + (PSP_STACK_WORDS as u32) * 4;
    let psp_before = psp_top - 4;
    let msp_before: u32;
    let psp_after: u32;
    let msp_after: u32;

    unsafe {
        let ccr = read_volatile(SCB_CCR as *const u32);
        write_volatile(SCB_CCR as *mut u32, ccr & !SCB_CCR_STKALIGN);
        let ccr_after = read_volatile(SCB_CCR as *const u32);
        core::arch::asm!(
            "msr psp, {top}",
            "mrs {msp_before}, msp",
            "movs {tmp}, #2",
            "msr control, {tmp}",
            "isb",
            "movw r0, #0x2468",
            "movt r0, #0xace0",
            "str {irq_bit}, [{iser}]",
            "movw {count}, #10000",
            "1:",
            "ldr {seen}, [{entry}]",
            "cmp {seen}, {mark}",
            "beq 2f",
            "subs {count}, #1",
            "bne 1b",
            "2:",
            "mrs {psp_after}, psp",
            "movs {tmp}, #0",
            "msr control, {tmp}",
            "isb",
            "mrs {msp_after}, msp",
            top = in(reg) psp_before,
            msp_before = out(reg) msp_before,
            psp_after = out(reg) psp_after,
            msp_after = out(reg) msp_after,
            tmp = out(reg) _,
            irq_bit = in(reg) FLEXCOMM0_IRQ_BIT,
            iser = in(reg) NVIC_ISER0 as u32,
            count = out(reg) _,
            seen = out(reg) _,
            entry = in(reg) addr_of!(IRQ_ENTRY) as u32,
            mark = in(reg) FLEXCOMM0_HANDLER_MARK,
            out("r0") _,
            options(nostack),
        );
        write_volatile(
            UNALIGNED_EXCEPTION_CONTEXT,
            u32::from(ccr_after & SCB_CCR_STKALIGN == 0),
        );
    }

    let exc_return = IRQ_EXC_RETURN.load(Ordering::Relaxed);
    let handler_entered = IRQ_ENTRY.load(Ordering::Relaxed) == FLEXCOMM0_HANDLER_MARK;
    let frame_r0 = IRQ_UNALIGNED_R0.load(Ordering::Relaxed);
    let frame_pc = IRQ_UNALIGNED_PC.load(Ordering::Relaxed);
    let frame_xpsr = IRQ_UNALIGNED_XPSR.load(Ordering::Relaxed);
    let frame_sp = IRQ_UNALIGNED_SP.load(Ordering::Relaxed);
    let frame_ok = frame_r0 == 0xACE0_2468
        && frame_pc & 1 == 0
        && (0x3800_0000..0x4000_0000).contains(&frame_pc)
        && frame_xpsr & (1 << 24) != 0
        && frame_xpsr & 0x1FF == 0
        && frame_xpsr & (1 << 9) == 0
        && frame_sp == psp_top - 9 * 4;
    let context = u32::from(exc_return == 0xFFFF_FFFD)
        | (u32::from(psp_after == psp_before) << 1)
        | (u32::from(msp_before == msp_after) << 2)
        | (u32::from(frame_ok) << 3)
        | (unsafe { read_volatile(UNALIGNED_EXCEPTION_CONTEXT) } << 4);
    unsafe {
        write_volatile(UNALIGNED_EXCEPTION_CONTEXT, context);
        write_volatile(UNALIGNED_EXCEPTION_FRAME_XPSR, frame_xpsr);
        write_volatile(UNALIGNED_EXCEPTION_FRAME_SP, frame_sp);
        write_volatile(UNALIGNED_EXCEPTION_PSP_AFTER, psp_after);
    }
    handler_entered && context == 0b1_1111
}

/// Deliver FLEXCOMM0 from Thread mode on a four-byte-aligned MSP with
/// STKALIGN enabled and disabled. Inspect both frames after exception return,
/// once the thread stack is restored.
fn exercise_irq_exception_on_misaligned_msp() -> bool {
    IRQ_CASE.store(4, Ordering::Relaxed);
    IRQ_ENTRY.store(0, Ordering::Relaxed);
    IRQ_EXC_RETURN.store(0, Ordering::Relaxed);

    let msp_top = (addr_of_mut!(PSP_STACK) as u32 + (PSP_STACK_WORDS as u32) * 4) & !7;
    let msp_thread = msp_top - 4;
    let msp_before: u32;
    let msp_after: u32;
    let msp_restored: u32;
    let ccr_after: u32;

    unsafe {
        let ccr = read_volatile(SCB_CCR as *const u32);
        write_volatile(SCB_CCR as *mut u32, ccr | SCB_CCR_STKALIGN);
        ccr_after = read_volatile(SCB_CCR as *const u32);
        core::arch::asm!(
            "mrs {saved_msp}, msp",
            "msr msp, {thread_msp}",
            "movw r0, #0x1357",
            "movt r0, #0x9bdf",
            "str {irq_bit}, [{iser}]",
            "movw {count}, #10000",
            "1:",
            "ldr {seen}, [{entry}]",
            "cmp {seen}, {mark}",
            "beq 2f",
            "subs {count}, #1",
            "bne 1b",
            "2:",
            "mrs {msp_after}, msp",
            "msr msp, {saved_msp}",
            "isb",
            "mrs {msp_restored}, msp",
            saved_msp = out(reg) msp_before,
            thread_msp = in(reg) msp_thread,
            msp_after = out(reg) msp_after,
            msp_restored = out(reg) msp_restored,
            irq_bit = in(reg) FLEXCOMM0_IRQ_BIT,
            iser = in(reg) NVIC_ISER0 as u32,
            count = out(reg) _,
            seen = out(reg) _,
            entry = in(reg) addr_of!(IRQ_ENTRY) as u32,
            mark = in(reg) FLEXCOMM0_HANDLER_MARK,
            out("r0") _,
            options(nostack),
        );
    }

    let frame_sp = msp_thread - 9 * 4;
    let frame = frame_sp as *const u32;
    let frame_r0 = unsafe { read_volatile(frame) };
    let frame_xpsr = unsafe { read_volatile(frame.add(7)) };
    let padding_word = unsafe { read_volatile(frame.add(8)) };
    let frame_ok = frame_r0 == 0x9BDF_1357
        && frame_xpsr & 0x0100_0200 == 0x0100_0200
        && frame_xpsr & 0x1FF == 0
        && padding_word == 0;
    let aligned_context = u32::from(ccr_after & SCB_CCR_STKALIGN != 0)
        | (u32::from(IRQ_EXC_RETURN.load(Ordering::Relaxed) == 0xFFFF_FFF9) << 1)
        | (u32::from(msp_after == msp_thread) << 2)
        | (u32::from(msp_restored == msp_before) << 3)
        | (u32::from(frame_ok) << 4);
    unsafe {
        write_volatile(MSP_EXCEPTION_CONTEXT, aligned_context);
        write_volatile(MSP_EXCEPTION_FRAME_R0, frame_r0);
        write_volatile(MSP_EXCEPTION_FRAME_XPSR, frame_xpsr);
        write_volatile(MSP_EXCEPTION_FRAME_SP, frame_sp);
        write_volatile(MSP_EXCEPTION_AFTER, msp_after);
        write_volatile(MSP_EXCEPTION_RESTORED, msp_restored);
        write_volatile(MSP_EXCEPTION_PADDING, padding_word);
    }

    // Repeat from the same four-byte-aligned Thread-mode MSP with CCR.STKALIGN
    // clear. This must leave an eight-word frame, with no xPSR alignment flag.
    IRQ_CASE.store(5, Ordering::Relaxed);
    IRQ_ENTRY.store(0, Ordering::Relaxed);
    IRQ_EXC_RETURN.store(0, Ordering::Relaxed);
    let msp_before: u32;
    let msp_after: u32;
    let msp_restored: u32;
    let ccr_after: u32;
    unsafe {
        let ccr = read_volatile(SCB_CCR as *const u32);
        write_volatile(SCB_CCR as *mut u32, ccr & !SCB_CCR_STKALIGN);
        ccr_after = read_volatile(SCB_CCR as *const u32);
        core::arch::asm!(
            "mrs {saved_msp}, msp",
            "msr msp, {thread_msp}",
            "movw r0, #0xace0",
            "movt r0, #0x2468",
            "str {irq_bit}, [{iser}]",
            "movw {count}, #10000",
            "1:",
            "ldr {seen}, [{entry}]",
            "cmp {seen}, {mark}",
            "beq 2f",
            "subs {count}, #1",
            "bne 1b",
            "2:",
            "mrs {msp_after}, msp",
            "msr msp, {saved_msp}",
            "isb",
            "mrs {msp_restored}, msp",
            saved_msp = out(reg) msp_before,
            thread_msp = in(reg) msp_thread,
            msp_after = out(reg) msp_after,
            msp_restored = out(reg) msp_restored,
            irq_bit = in(reg) FLEXCOMM0_IRQ_BIT,
            iser = in(reg) NVIC_ISER0 as u32,
            count = out(reg) _,
            seen = out(reg) _,
            entry = in(reg) addr_of!(IRQ_ENTRY) as u32,
            mark = in(reg) FLEXCOMM0_HANDLER_MARK,
            out("r0") _,
            options(nostack),
        );
    }

    let frame_sp = msp_thread - 8 * 4;
    let frame = frame_sp as *const u32;
    let frame_r0 = unsafe { read_volatile(frame) };
    let frame_pc = unsafe { read_volatile(frame.add(6)) };
    let frame_xpsr = unsafe { read_volatile(frame.add(7)) };
    let frame_ok = frame_r0 == 0x2468_ACE0
        && frame_pc & 1 == 0
        && (0x3800_0000..0x4000_0000).contains(&frame_pc)
        && frame_xpsr & (1 << 24) != 0
        && frame_xpsr & 0x1FF == 0
        && frame_xpsr & (1 << 9) == 0
        && frame_sp == msp_thread - 8 * 4;
    let unaligned_context = u32::from(ccr_after & SCB_CCR_STKALIGN == 0)
        | (u32::from(IRQ_EXC_RETURN.load(Ordering::Relaxed) == 0xFFFF_FFF9) << 1)
        | (u32::from(msp_after == msp_thread) << 2)
        | (u32::from(msp_restored == msp_before) << 3)
        | (u32::from(frame_ok) << 4);

    puts("\r\nmsp_noalign_context=");
    hex(unaligned_context);
    puts(" msp_noalign_r0=");
    hex(frame_r0);
    puts(" msp_noalign_xpsr=");
    hex(frame_xpsr & (0x0100_0000 | 0x0000_0200 | 0x0000_01FF));
    puts(" msp_noalign_sp=");
    hex(frame_sp);
    puts(" msp_noalign_after=");
    hex(msp_after);
    puts(" restore=");
    putc(if msp_restored == msp_before {
        b'1'
    } else {
        b'0'
    });
    puts("\r\n");

    let context = aligned_context | (unaligned_context << 5);
    unsafe { write_volatile(MSP_EXCEPTION_CONTEXT, context) };
    IRQ_ENTRY.load(Ordering::Relaxed) == FLEXCOMM0_HANDLER_MARK
        && aligned_context == 0b1_1111
        && unaligned_context == 0b1_1111
}

fn wait_nvic_irq_pending(expected: bool) -> bool {
    for _ in 0..10_000 {
        if nvic_irq_pending() == expected {
            return true;
        }
    }
    false
}

fn putc(b: u8) {
    while rd(LPUART_STAT) & STAT_TDRE == 0 {}
    wr(LPUART_DATA, b as u32);
}

fn puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

fn hex(v: u32) {
    for i in (0..8).rev() {
        let n = ((v >> (i * 4)) & 0xF) as u8;
        putc(if n < 10 { b'0' + n } else { b'a' + n - 10 });
    }
}

/// Exercise the imported LPUART RX FIFO, LP_FLEXCOMM receive status, CPU0
/// NVIC pending line, and exception delivery (FLEXCOMM0 IRQ 7).
fn smoke_lpuart_rx() -> bool {
    // Leave TE on for the later console output, but disable TIE so the shared
    // FLEXCOMM line reflects the receive interrupt only. Clear the earlier TX
    // pending bit before checking the receive gates.
    wr(LPUART_CTRL, CTRL_TE);
    clear_nvic_irq_pending();
    let irq_isolated = wait_nvic_irq_pending(false);
    let receiver_off =
        rd(LPUART_STAT) & STAT_RDRF == 0 && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0 && irq_isolated;

    // Two injected bytes need the modeled four-entry FIFO enabled. Keep RIE
    // clear while receiving so data-ready and interrupt status are distinct.
    wr(LPUART_FIFO, rd(LPUART_FIFO) | FIFO_RXFE);
    wr(LPUART_CTRL, CTRL_TE | CTRL_RE);
    let mut queued = false;
    for _ in 0..1_000_000 {
        if rd(LPUART_WATER) >> WATER_RXCOUNT_SHIFT >= 2 {
            queued = true;
            break;
        }
    }

    let rx_masked = queued
        && rd(LPUART_STAT) & STAT_RDRF != 0
        && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0
        && wait_nvic_irq_pending(false);
    wr(LPUART_CTRL, CTRL_TE | CTRL_RE | CTRL_RIE);
    let rx_asserted = rd(LPFC_ISTAT) & ISTAT_UARTRX != 0 && wait_nvic_irq_pending(true);
    clear_nvic_irq_pending();
    let rx_repend = wait_nvic_irq_pending(true);

    // Enable the already-pending external IRQ from Thread/PSP and require the
    // Cortex-M exception path to call DefaultHandler. The handler disables IRQ
    // 7 before returning so the active RX level cannot retrigger it. It records
    // IPSR and EXC_RETURN; the interrupted thread then checks PSP and MSP.
    let irq_psp_context_restored = exercise_irq_exception_on_psp();
    let irq_handler_entered = IRQ_ENTRY.load(Ordering::Relaxed) == FLEXCOMM0_HANDLER_MARK;
    let irq_exc_return = IRQ_EXC_RETURN.load(Ordering::Relaxed);
    let irq_padded_psp_context_restored = exercise_irq_exception_on_misaligned_psp();
    let irq_unaligned_psp_context_restored = exercise_irq_exception_without_stack_alignment();
    let irq_padded_msp_context_restored = exercise_irq_exception_on_misaligned_msp();

    let first = rd(LPUART_DATA) & 0xFF;
    let second_still_queued = rd(LPUART_STAT) & STAT_RDRF != 0;
    wr(LPUART_CTRL, CTRL_TE | CTRL_RE);
    let rx_masked_with_data = second_still_queued
        && rd(LPUART_STAT) & STAT_RDRF != 0
        && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0;
    clear_nvic_irq_pending();
    let irq_masked_released = rx_masked_with_data && wait_nvic_irq_pending(false);
    wr(LPUART_CTRL, CTRL_TE | CTRL_RE | CTRL_RIE);
    let rx_unmasked = rd(LPFC_ISTAT) & ISTAT_UARTRX != 0 && wait_nvic_irq_pending(true);

    let second = rd(LPUART_DATA) & 0xFF;
    clear_nvic_irq_pending();
    let irq_released = wait_nvic_irq_pending(false);
    let released =
        rd(LPUART_STAT) & STAT_RDRF == 0 && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0 && irq_released;
    let ok = receiver_off
        && queued
        && rx_masked
        && rx_asserted
        && rx_repend
        && irq_handler_entered
        && irq_psp_context_restored
        && irq_padded_psp_context_restored
        && irq_unaligned_psp_context_restored
        && irq_padded_msp_context_restored
        && first == b'R' as u32
        && second_still_queued
        && rx_masked_with_data
        && irq_masked_released
        && rx_unmasked
        && second == b'S' as u32
        && released;

    puts("rx lpuart0 re_off=");
    putc(if receiver_off { b'1' } else { b'0' });
    puts(" queued=");
    putc(if queued { b'2' } else { b'0' });
    puts(" masked=");
    putc(if rx_masked { b'1' } else { b'0' });
    puts(" asserted=");
    putc(if rx_asserted { b'1' } else { b'0' });
    puts(" masked_data=");
    putc(if rx_masked_with_data { b'1' } else { b'0' });
    puts(" unmasked=");
    putc(if rx_unmasked { b'1' } else { b'0' });
    puts(" data=");
    hex(first);
    putc(b',');
    hex(second);
    puts(" released=");
    putc(if released { b'1' } else { b'0' });
    puts(" irq=");
    putc(if rx_asserted { b'1' } else { b'0' });
    putc(if rx_repend { b'1' } else { b'0' });
    putc(if irq_masked_released { b'1' } else { b'0' });
    putc(if rx_unmasked { b'1' } else { b'0' });
    putc(if irq_released { b'1' } else { b'0' });
    puts(" handler=");
    putc(if irq_handler_entered { b'1' } else { b'0' });
    puts(" ipsr=");
    hex(IRQ_ENTRY.load(Ordering::Relaxed) >> 16);
    puts(" exc_return=");
    hex(irq_exc_return);
    puts(" psp_context=");
    putc(if irq_psp_context_restored { b'1' } else { b'0' });
    puts(" frame_r0=");
    hex(IRQ_STACKED_R0.load(Ordering::Relaxed));
    puts(" frame_pc=");
    hex(IRQ_STACKED_PC.load(Ordering::Relaxed));
    puts(" frame_xpsr=");
    hex(IRQ_STACKED_XPSR.load(Ordering::Relaxed));
    puts(" padded_context=");
    hex(unsafe { read_volatile(PADDED_EXCEPTION_CONTEXT) });
    puts(" padded_xpsr=");
    hex(IRQ_PADDED_XPSR.load(Ordering::Relaxed));
    puts(" padded_sp=");
    hex(IRQ_PADDED_SP.load(Ordering::Relaxed));
    puts(" unaligned_context=");
    hex(unsafe { read_volatile(UNALIGNED_EXCEPTION_CONTEXT) });
    puts(" unaligned_xpsr=");
    hex(IRQ_UNALIGNED_XPSR.load(Ordering::Relaxed));
    puts(" unaligned_sp=");
    hex(IRQ_UNALIGNED_SP.load(Ordering::Relaxed));
    puts(" msp_context=");
    hex(unsafe { read_volatile(MSP_EXCEPTION_CONTEXT) });
    puts(" msp_r0=");
    hex(unsafe { read_volatile(MSP_EXCEPTION_FRAME_R0) });
    puts(" msp_xpsr=");
    hex(unsafe { read_volatile(MSP_EXCEPTION_FRAME_XPSR) });
    puts(" msp_sp=");
    hex(unsafe { read_volatile(MSP_EXCEPTION_FRAME_SP) });
    puts(" msp_after=");
    hex(unsafe { read_volatile(MSP_EXCEPTION_AFTER) });
    puts(" msp_restored=");
    hex(unsafe { read_volatile(MSP_EXCEPTION_RESTORED) });
    puts(" msp_pad=");
    hex(unsafe { read_volatile(MSP_EXCEPTION_PADDING) });
    puts(if ok { " ok\r\n" } else { " FAIL\r\n" });
    ok
}

#[entry]
fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "mul {product}, {product}, {right}",
            product = inout(reg) 17u32 => product,
            right = in(reg) 19u32,
            options(nomem, nostack),
        );
        write_volatile(RESULT, product);
    }

    let pselid_reset = rd(LPFC_PSELID);
    wr(LPFC_PSELID, PSELID_UART);
    let pselid_uart = rd(LPFC_PSELID);
    wr(LPFC_PSELID, PSELID_UART | PSELID_LOCK);
    wr(LPFC_PSELID, 0);
    let pselid_locked = rd(LPFC_PSELID);

    // 80 MHz functional clock, OSR=16, SBR=26; the smoke checks TX/RX and
    // register behavior, not a board-level baud-rate claim.
    wr(LPUART_BAUD, (15 << 24) | 26);
    wr(LPUART_CTRL, CTRL_TIE_TE);
    let istat = rd(LPFC_ISTAT);
    puts("MIMXRT700 LP_FLEXCOMM0 LPUART SMOKE\r\n");

    let verid = rd(LPUART_VERID);
    let param = rd(LPUART_PARAM);
    let baud = rd(LPUART_BAUD);
    let ctrl = rd(LPUART_CTRL);
    let mut pass = 0u32;
    if pselid_reset == PSELID_ID_PRESENT
        && pselid_uart == (PSELID_ID_PRESENT | PSELID_UART)
        && pselid_locked == (PSELID_ID_PRESENT | PSELID_UART | PSELID_LOCK)
    {
        pass |= 1;
    }
    if verid == 0x0401_0003 && param == 0x0202 {
        pass |= 1 << 1;
    }
    if baud == ((15 << 24) | 26) && ctrl == CTRL_TIE_TE && istat & 1 != 0 {
        pass |= 1 << 2;
    }
    if product == 17 * 19 {
        pass |= 1 << 3;
    }
    if smoke_lpuart_rx() {
        pass |= 1 << 4;
    }
    if IRQ_ENTRY.load(Ordering::Relaxed) == FLEXCOMM0_HANDLER_MARK {
        pass |= 1 << 5;
    }
    if unsafe { read_volatile(EXCEPTION_CONTEXT) } & 0b111 == 0b111 {
        pass |= 1 << 6;
    }
    if unsafe { read_volatile(EXCEPTION_CONTEXT) } == 0b1111 {
        pass |= 1 << 7;
    }
    if unsafe { read_volatile(PADDED_EXCEPTION_CONTEXT) } == 0b1111 {
        pass |= 1 << 8;
    }
    if unsafe { read_volatile(UNALIGNED_EXCEPTION_CONTEXT) } == 0b1_1111 {
        pass |= 1 << 9;
    }
    if unsafe { read_volatile(MSP_EXCEPTION_CONTEXT) } == 0b11_1111_1111 {
        pass |= 1 << 10;
    }

    puts("flexcomm0 pselid=");
    hex(pselid_reset);
    puts("->");
    hex(pselid_uart);
    puts("->");
    hex(pselid_locked);
    puts("\r\nlpuart0 verid=");
    hex(verid);
    puts(" param=");
    hex(param);
    puts(" baud=");
    hex(baud);
    puts(" ctrl=");
    hex(ctrl);
    puts(" istat=");
    hex(istat);
    puts("\r\npass=");
    hex(pass);
    puts("\r\n");
    while rd(LPUART_STAT) & STAT_TC == 0 {}
    unsafe { write_volatile(PASS, 0x7000_0000 | pass) };

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn == FLEXCOMM0_IRQN {
        let exc_return: u32;
        let ipsr: u32;
        let psp: u32;
        unsafe {
            core::arch::asm!(
                "mov {exc_return}, lr",
                exc_return = out(reg) exc_return,
                options(nomem, nostack, preserves_flags),
            );
            core::arch::asm!(
                "mrs {ipsr}, IPSR",
                ipsr = out(reg) ipsr,
                options(nomem, nostack, preserves_flags),
            );
            core::arch::asm!(
                "mrs {psp}, PSP",
                psp = out(reg) psp,
                options(nomem, nostack, preserves_flags),
            );
        }
        let frame = psp as *const u32;
        let stacked_r0 = unsafe { read_volatile(frame) };
        let stacked_pc = unsafe { read_volatile(frame.add(6)) };
        let stacked_xpsr = unsafe { read_volatile(frame.add(7)) };
        IRQ_EXC_RETURN.store(exc_return, Ordering::Relaxed);
        IRQ_ENTRY.store((ipsr << 16) | (irqn as u32 + 1), Ordering::Relaxed);
        let irq_case = IRQ_CASE.load(Ordering::Relaxed);
        if irq_case == 4 || irq_case == 5 {
            // MSP frames are inspected by the interrupted thread after return;
            // the Rust handler prologue itself uses MSP.
        } else {
            let psp: u32;
            core::arch::asm!(
                "mrs {psp}, PSP",
                psp = out(reg) psp,
                options(nomem, nostack, preserves_flags),
            );
            let frame = psp as *const u32;
            let stacked_r0 = read_volatile(frame);
            let stacked_pc = read_volatile(frame.add(6));
            let stacked_xpsr = read_volatile(frame.add(7));
            if irq_case == 2 {
                IRQ_PADDED_R0.store(stacked_r0, Ordering::Relaxed);
                IRQ_PADDED_PC.store(stacked_pc, Ordering::Relaxed);
                IRQ_PADDED_XPSR.store(stacked_xpsr, Ordering::Relaxed);
                IRQ_PADDED_SP.store(psp, Ordering::Relaxed);
            } else if irq_case == 3 {
                IRQ_UNALIGNED_R0.store(stacked_r0, Ordering::Relaxed);
                IRQ_UNALIGNED_PC.store(stacked_pc, Ordering::Relaxed);
                IRQ_UNALIGNED_XPSR.store(stacked_xpsr, Ordering::Relaxed);
                IRQ_UNALIGNED_SP.store(psp, Ordering::Relaxed);
            } else {
                IRQ_STACKED_R0.store(stacked_r0, Ordering::Relaxed);
                IRQ_STACKED_PC.store(stacked_pc, Ordering::Relaxed);
                IRQ_STACKED_XPSR.store(stacked_xpsr, Ordering::Relaxed);
                IRQ_STACKED_SP.store(psp, Ordering::Relaxed);
            }
        }
        unsafe { write_volatile(NVIC_ICER0 as *mut u32, FLEXCOMM0_IRQ_BIT) };
    }
}
