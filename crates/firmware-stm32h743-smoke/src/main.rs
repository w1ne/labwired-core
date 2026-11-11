#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

#[cfg(feature = "stm32h743-exception-frame")]
use core::arch::asm;

#[cfg(feature = "stm32h743-exception-frame")]
const SCB_VTOR: *mut u32 = 0xE000_ED08 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const SCB_CCR: *mut u32 = 0xE000_ED14 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const SCB_SHCSR: *mut u32 = 0xE000_ED24 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const SCB_CFSR: *mut u32 = 0xE000_ED28 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const SCB_HFSR: *mut u32 = 0xE000_ED2C as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const SCB_SHPR1: *mut u32 = 0xE000_ED18 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const SCB_CCR_STKALIGN: u32 = 1 << 9;
#[cfg(feature = "stm32h743-exception-frame")]
const SCB_CPACR: *mut u32 = 0xE000_ED88 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const CPACR_CP10_CP11: u32 = 0x00F0_0000;
#[cfg(feature = "stm32h743-exception-frame")]
const CPACR_PRIVILEGED_ONLY: u32 = 0x0050_0000;
#[cfg(feature = "stm32h743-exception-frame")]
const CPACR_RESERVED_ACCESS: u32 = (0b10 << 20) | (0b10 << 22);
#[cfg(feature = "stm32h743-exception-frame")]
const SHCSR_USGFAULTENA: u32 = 1 << 18;
#[cfg(feature = "stm32h743-exception-frame")]
const CFSR_UFSR_NOCP: u32 = 1 << 19;
#[cfg(feature = "stm32h743-exception-frame")]
const HFSR_FORCED: u32 = 1 << 30;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_ISER0: *mut u32 = 0xE000_E100 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_ICER0: *mut u32 = 0xE000_E180 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_ISPR0: *mut u32 = 0xE000_E200 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_ICPR0: *mut u32 = 0xE000_E280 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_IABR0: *const u32 = 0xE000_E300 as *const u32;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_IPR0: *mut u8 = 0xE000_E400 as *mut u8;
#[cfg(feature = "stm32h743-exception-frame")]
const EXCEPTION_IRQ: u32 = 5;
#[cfg(feature = "stm32h743-exception-frame")]
const EXCEPTION_IRQ_BIT: u32 = 1 << EXCEPTION_IRQ;
#[cfg(feature = "stm32h743-exception-frame")]
const EXCEPTION_VECTOR_INDEX: usize = 16 + EXCEPTION_IRQ as usize;
#[cfg(feature = "stm32h743-exception-frame")]
const SVC_VECTOR_INDEX: usize = 11;
#[cfg(feature = "stm32h743-exception-frame")]
const NESTED_IRQ: u32 = 6;
#[cfg(feature = "stm32h743-exception-frame")]
const NESTED_IRQ_BIT: u32 = 1 << NESTED_IRQ;
#[cfg(feature = "stm32h743-exception-frame")]
const NESTED_VECTOR_INDEX: usize = 16 + NESTED_IRQ as usize;

#[cfg(feature = "stm32h743-exception-frame")]
#[repr(C, align(1024))]
struct RamVectorTable([u32; 64]);

#[cfg(feature = "stm32h743-exception-frame")]
static mut RAM_VECTOR_TABLE: RamVectorTable = RamVectorTable([0; 64]);

// Handler scratch and the stacked frame captured before exception return.
#[cfg(feature = "stm32h743-exception-frame")]
static mut EXCEPTION_RECORD: [u32; 18] = [0; 18];

#[cfg(feature = "stm32h743-exception-frame")]
static mut CPACR_FAULT_RECORD: [u32; 5] = [0; 5];

#[cfg(feature = "stm32h743-exception-frame")]
static mut CPACR_HANDLER_RECORD: [u32; 2] = [0; 2];

#[cfg(feature = "stm32h743-exception-frame")]
static mut CPACR_PRIORITY_IRQ_RECORD: [u32; 2] = [0; 2];

#[cfg(feature = "stm32h743-exception-frame")]
static FP_SENTINELS: [u32; 16] = [
    0x1111_0000,
    0x2222_0001,
    0x3333_0002,
    0x4444_0003,
    0x5555_0004,
    0x6666_0005,
    0x7777_0006,
    0x8888_0007,
    0x9999_0008,
    0xAAAA_0009,
    0xBBBB_000A,
    0xCCCC_000B,
    0xDDDD_000C,
    0xEEEE_000D,
    0xFFFF_000E,
    0xABCD_000F,
];

#[cfg(feature = "stm32h743-exception-frame")]
static mut FP_CAPTURE: [u32; 17] = [0; 17];

#[cfg(feature = "stm32h743-exception-frame")]
static mut FP_LAZY_TOUCH: u32 = 0;

#[cfg(feature = "stm32h743-exception-frame")]
static mut FP_LAZY_NEST: u32 = 0;

#[cfg(feature = "stm32h743-exception-frame")]
static mut FP_LAZY_NEST_TOUCH: u32 = 0;

#[cfg(feature = "stm32h743-exception-frame")]
static mut FP_LAZY_FPCCR: [u32; 2] = [0; 2];

#[cfg(feature = "stm32h743-exception-frame")]
static mut FP_NEST_RECORD: [u32; 6] = [0; 6];

#[cfg(feature = "stm32h743-exception-frame")]
static FP_LAZY_CLOBBER: [u32; 16] = [
    0xC000_0000,
    0xC000_0001,
    0xC000_0002,
    0xC000_0003,
    0xC000_0004,
    0xC000_0005,
    0xC000_0006,
    0xC000_0007,
    0xC000_0008,
    0xC000_0009,
    0xC000_000A,
    0xC000_000B,
    0xC000_000C,
    0xC000_000D,
    0xC000_000E,
    0xC000_000F,
];

#[cfg(feature = "stm32h743-exception-frame")]
const FPU_FPCAR: *const u32 = 0xE000_EF38 as *const u32;
#[cfg(feature = "stm32h743-exception-frame")]
const FPU_FPCCR: *mut u32 = 0xE000_EF34 as *mut u32;

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
const USART_UE: u32 = 1 << 0;
const USART_TE: u32 = 1 << 3;
const USART_RE: u32 = 1 << 2;
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

#[cfg(feature = "nucleo-h753zi-rx")]
fn check_uart_rx_enable_gates() -> bool {
    // Start-injected bytes are still queued on the wire. UE alone cannot
    // receive them while RE is clear, and RE alone cannot receive them while
    // UE is clear. Keep TE enabled so the UART remains otherwise configured.
    unsafe { write_volatile(USART_CR1, USART_UE | USART_TE) };
    let mut re_off_clear = true;
    for _ in 0..10_000 {
        if unsafe { read_volatile(USART_ISR) } & RXNE != 0 {
            re_off_clear = false;
            break;
        }
    }

    unsafe { write_volatile(USART_CR1, USART_TE | USART_RE) };
    let mut ue_off_clear = true;
    for _ in 0..10_000 {
        if unsafe { read_volatile(USART_ISR) } & RXNE != 0 {
            ue_off_clear = false;
            break;
        }
    }

    unsafe { write_volatile(USART_CR1, USART_UE | USART_TE | USART_RE) };
    re_off_clear && ue_off_clear
}

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

#[cfg(feature = "stm32h743-exception-frame")]
fn send_hex32(value: u32) {
    for shift in (0..8).rev() {
        let digit = ((value >> (shift * 4)) & 0xF) as u8;
        putc(if digit < 10 {
            b'0' + digit
        } else {
            b'A' + digit - 10
        });
    }
}

/// Execute one FP load through a normal function body so Rust applies the
/// target's FP-register features while assembling the instruction.
#[cfg(feature = "stm32h743-exception-frame")]
unsafe fn touch_fp_in_handler() {
    let values = core::ptr::addr_of!(FP_LAZY_CLOBBER).cast::<u32>();
    asm!(
        "vldmia r0!, {{s0-s15}}",
        inout("r0") values => _,
        options(nostack)
    );
}

/// Capture the hardware-created Cortex-M7 frame before a compiler prologue
/// changes the active stack. r4/r5 are saved because they are not part of the
/// basic exception frame; caller-saved registers are restored by exception
/// return.
#[cfg(feature = "stm32h743-exception-frame")]
#[unsafe(naked)]
unsafe extern "C" fn smoke_exception_handler() {
    core::arch::naked_asm!(
        "mov r2, lr",
        "tst r2, #4",
        "ite eq",
        "mrseq r0, msp",
        "mrsne r0, psp",
        "push {{r4-r5}}",
        "ldr r1, ={record}",
        "str r2, [r1, #32]",
        "str r0, [r1, #36]",
        "mov r4, r0",
        "mov r5, r2",
        // Record LSPACT at handler entry, optionally touch the FPU, then
        // record it again to distinguish a reserved lazy frame from a save
        // triggered by the handler's first FP instruction.
        "ldr r3, ={fpccr_record}",
        "ldr r2, =0xE000EF34",
        "ldr r0, [r2]",
        "str r0, [r3]",
        "ldr r2, ={fp_lazy_nest}",
        "ldr r2, [r2]",
        "cbz r2, 5f",
        // IRQ 6 has higher priority than IRQ 5. It must preserve the pending
        // outer lazy save while it takes and returns from a basic frame.
        "ldr r2, =0xE000E200",
        "movs r3, #0x40",
        "str r3, [r2]",
        "5:",
        "ldr r2, ={fp_lazy_touch}",
        "ldr r2, [r2]",
        "cbz r2, 4f",
        "bl {fp_lazy_touch_fn}",
        "4:",
        "mov lr, r5",
        "ldr r1, ={record}",
        "ldr r3, ={fpccr_record}",
        "ldr r2, =0xE000EF34",
        "ldr r0, [r2]",
        "str r0, [r3, #4]",
        "movs r2, #8",
        "2:",
        "ldr r3, [r4]",
        "str r3, [r1]",
        "adds r4, #4",
        "adds r1, #4",
        "subs r2, #1",
        "bne 2b",
        // Extended FP frames retain the basic core frame at words 0..7 and
        // append S0-S15/FPSCR beginning at offset 0x20.
        "ldr r3, ={record}",
        "ldr r3, [r3, #32]",
        "tst r3, #16",
        "bne 3f",
        "ldr r3, [r4, #0]",
        "str r3, [r1, #24]",
        "ldr r3, [r4, #4]",
        "str r3, [r1, #28]",
        "ldr r3, [r4, #60]",
        "str r3, [r1, #32]",
        "ldr r3, [r4, #64]",
        "str r3, [r1, #36]",
        "3:",
        "mrs r0, ipsr",
        "ldr r1, ={record}",
        "str r0, [r1, #40]",
        "ldr r2, =0xE000E300",
        "ldr r0, [r2]",
        "str r0, [r1, #44]",
        "ldr r2, =0xE000E200",
        "ldr r0, [r2]",
        "str r0, [r1, #48]",
        "pop {{r4-r5}}",
        "bx lr",
        record = sym EXCEPTION_RECORD,
        fpccr_record = sym FP_LAZY_FPCCR,
        fp_lazy_nest = sym FP_LAZY_NEST,
        fp_lazy_touch = sym FP_LAZY_TOUCH,
        fp_lazy_touch_fn = sym touch_fp_in_handler,
    );
}

/// Observe the pending outer lazy FP save from a higher-priority nested IRQ.
#[cfg(feature = "stm32h743-exception-frame")]
#[unsafe(naked)]
unsafe extern "C" fn smoke_nested_exception_handler() {
    core::arch::naked_asm!(
        "ldr r0, =0xE000EF34",
        "ldr r1, [r0]",
        "ldr r0, ={record}",
        "str r1, [r0, #0]",
        "ldr r1, =0xE000EF38",
        "ldr r1, [r1]",
        "str r1, [r0, #4]",
        "mrs r1, ipsr",
        "str r1, [r0, #8]",
        "ldr r1, =0xE000E300",
        "ldr r1, [r1]",
        "str r1, [r0, #12]",
        "mov r1, lr",
        "str r1, [r0, #16]",
        "ldr r0, ={touch}",
        "ldr r0, [r0]",
        "cbz r0, 1f",
        "bl {fp_lazy_touch_fn}",
        "1:",
        "ldr r0, =0xE000EF34",
        "ldr r1, [r0]",
        "ldr r0, ={record}",
        "str r1, [r0, #20]",
        "ldr r1, [r0, #16]",
        "mov lr, r1",
        "bx lr",
        record = sym FP_NEST_RECORD,
        touch = sym FP_LAZY_NEST_TOUCH,
        fp_lazy_touch_fn = sym touch_fp_in_handler,
    );
}

/// Record and skip a denied VFP instruction under UsageFault or escalated
/// HardFault. The core-register frame begins at SP; an extended FP frame
/// follows it.
#[cfg(feature = "stm32h743-exception-frame")]
#[unsafe(naked)]
unsafe extern "C" fn nocp_fault_handler() {
    core::arch::naked_asm!(
        "mov r0, lr",
        "tst r0, #4",
        "ite eq",
        "mrseq r0, msp",
        "mrsne r0, psp",
        "ldr r1, ={record}",
        "ldr r2, [r0, #24]",
        "str r2, [r1]",
        "add r2, r2, #4",
        "str r2, [r0, #24]",
        "ldr r2, ={cfsr}",
        "ldr r2, [r2]",
        "str r2, [r1, #4]",
        "mrs r2, ipsr",
        "str r2, [r1, #8]",
        "mov r2, lr",
        "str r2, [r1, #12]",
        "ldr r2, ={hfsr}",
        "ldr r2, [r2]",
        "str r2, [r1, #16]",
        "bx lr",
        record = sym CPACR_FAULT_RECORD,
        cfsr = const 0xE000_ED28usize,
        hfsr = const 0xE000_ED2Cusize,
    );
}

/// Exercise CPACR denial from IRQ 5 while UsageFault is enabled at a
/// priority that cannot preempt the active IRQ. The denied instruction must
/// therefore escalate to HardFault, which can preempt IRQ 5.
#[cfg(feature = "stm32h743-exception-frame")]
#[unsafe(naked)]
unsafe extern "C" fn cpacr_priority_irq_handler() {
    core::arch::naked_asm!(
        "push {{r4, lr}}",
        "mrs r0, ipsr",
        "ldr r1, ={record}",
        "str r0, [r1]",
        "movs r0, #1",
        "str r0, [r1, #4]",
        "bl {attempt}",
        "pop {{r4, lr}}",
        "bx lr",
        record = sym CPACR_PRIORITY_IRQ_RECORD,
        attempt = sym attempt_privileged_fp,
    );
}

/// Execute a real VFP round trip from SVC Handler mode. The caller enters
/// SVC from unprivileged Thread mode while CP10/CP11 are privileged-only.
#[cfg(feature = "stm32h743-exception-frame")]
#[unsafe(naked)]
unsafe extern "C" fn cpacr_svc_handler() {
    core::arch::naked_asm!(
        // Restrict FP access from privileged Handler mode before the VFP
        // round trip, even though the interrupted Thread mode is unprivileged.
        "ldr r1, =0xE000ED88",
        "ldr r2, =0x00500000",
        "str r2, [r1]",
        "dsb",
        "isb",
        "push {{r4-r5}}",
        "mov r4, lr",
        "bl {roundtrip}",
        "ldr r1, ={record}",
        "str r0, [r1]",
        "mrs r0, ipsr",
        "str r0, [r1, #4]",
        "mov lr, r4",
        "pop {{r4-r5}}",
        "bx lr",
        roundtrip = sym cpacr_handler_fp_roundtrip,
        record = sym CPACR_HANDLER_RECORD,
    );
}

/// Keep VFP instructions in a normal function so target FP features apply.
#[cfg(feature = "stm32h743-exception-frame")]
#[inline(never)]
unsafe fn cpacr_handler_fp_roundtrip() -> u32 {
    let result: u32;
    asm!(
        "movs r0, #0xc3",
        "vmov s31, r0",
        "movs r0, #0",
        "vmov r0, s31",
        out("r0") result,
        out("s31") _,
        options(nostack)
    );
    result
}

/// Enter unprivileged Thread mode and issue SVC so the installed handler can
/// prove Handler-mode CPACR access and this function can confirm exception
/// return restores the interrupted Thread privilege state.
#[cfg(feature = "stm32h743-exception-frame")]
#[inline(never)]
unsafe fn attempt_unprivileged_svc() -> u32 {
    let thread_unprivileged: u32;
    asm!(
        "mrs r0, control",
        "bic r0, r0, #2",
        "orr r0, r0, #1",
        "msr control, r0",
        "isb",
        "svc #0",
        "mrs r0, control",
        "and r0, r0, #1",
        out("r0") thread_unprivileged,
        options(nostack)
    );
    thread_unprivileged
}

/// Switch Thread mode to unprivileged and execute one four-byte VFP
/// instruction. A correct CPACR implementation returns here via the installed
/// UsageFault handler, which advances the stacked PC past the denied access.
#[cfg(feature = "stm32h743-exception-frame")]
#[inline(never)]
unsafe fn attempt_unprivileged_fp() {
    asm!(
        "mrs r0, control",
        "bic r0, r0, #2",
        "orr r0, r0, #1",
        "msr control, r0",
        "isb",
        "vmov.f32 s31, s31",
        out("r0") _,
        options(nostack)
    );
}

/// Execute one VFP instruction in privileged Thread mode so CPACR=00 can be
/// distinguished from the privileged-only CP10/CP11 grant.
#[cfg(feature = "stm32h743-exception-frame")]
#[inline(never)]
unsafe fn attempt_privileged_fp() {
    asm!(
        "vmov.f32 s31, s31",
        out("s31") _,
        options(nostack)
    );
}

/// Verify that privileged-only CP10/CP11 access permits an actual FP
/// round-trip while Thread mode remains privileged. The integer result
/// proves both VFP instructions retired.
#[cfg(feature = "stm32h743-exception-frame")]
#[inline(never)]
unsafe fn privileged_only_fp_roundtrip() -> u32 {
    let result: u32;
    asm!(
        "movs r0, #0xa5",
        "vmov s31, r0",
        "movs r0, #0",
        "vmov r0, s31",
        out("r0") result,
        out("s31") _,
        options(nostack)
    );
    result
}

/// Switch Thread mode to unprivileged and verify that full CP10/CP11 access
/// permits a VFP register round trip. The integer result proves both VFP
/// instructions retired instead of being skipped by the UsageFault handler.
#[cfg(feature = "stm32h743-exception-frame")]
#[inline(never)]
unsafe fn unprivileged_full_access_fp_roundtrip() -> u32 {
    let result: u32;
    asm!(
        "mrs r0, control",
        "bic r0, r0, #2",
        "orr r0, r0, #1",
        "msr control, r0",
        "isb",
        "movs r0, #0x5a",
        "vmov s31, r0",
        "movs r0, #0",
        "vmov r0, s31",
        out("r0") result,
        out("s31") _,
        options(nostack)
    );
    result
}

/// Pend IRQ 5 after setting sentinels in every caller-saved register which
/// Cortex-M stacks automatically. The recorded pre-exception stack includes
/// the r4/r7 save on MSP, or is the unchanged PSP when Thread mode uses PSP.
#[cfg(feature = "stm32h743-exception-frame")]
#[unsafe(naked)]
unsafe extern "C" fn pend_exception_irq_with_sentinels() {
    core::arch::naked_asm!(
        "push {{r4, r7}}",
        "mrs r7, control",
        "tst r7, #2",
        "ite eq",
        "mrseq r7, msp",
        "mrsne r7, psp",
        "ldr r4, ={record}",
        "str r7, [r4, #52]",
        "ldr r7, =0xE000E200",
        "movs r4, #0x20",
        "movs r0, #0x11",
        "movs r1, #0x22",
        "movs r2, #0x33",
        "movs r3, #0x44",
        "movs r4, #0x55",
        "mov r12, r4",
        "movs r4, #0x20",
        "str r4, [r7]",
        "pop {{r4, r7}}",
        "bx lr",
        record = sym EXCEPTION_RECORD,
    );
}

/// Switch Thread mode to a dedicated PSP, pend IRQ 5, then restore the
/// caller's CONTROL and callee-saved registers after exception return.
#[cfg(feature = "stm32h743-exception-frame")]
#[unsafe(naked)]
unsafe extern "C" fn pend_exception_irq_with_psp_sentinels(psp_top: u32) {
    core::arch::naked_asm!(
        "push {{r4-r7}}",
        "mrs r6, control",
        "mov r7, r0",
        "msr psp, r7",
        "movs r7, #2",
        "msr control, r7",
        "isb",
        "mrs r7, psp",
        "ldr r5, ={record}",
        "str r7, [r5, #52]",
        "ldr r7, =0xE000E200",
        "movs r4, #0x20",
        "movs r0, #0x11",
        "movs r1, #0x22",
        "movs r2, #0x33",
        "movs r3, #0x44",
        "movs r4, #0x55",
        "mov r12, r4",
        "movs r4, #0x20",
        "str r4, [r7]",
        "msr control, r6",
        "isb",
        "pop {{r4-r7}}",
        "bx lr",
        record = sym EXCEPTION_RECORD,
    );
}

/// Establish caller-owned FP state before taking the next exception.
#[cfg(feature = "stm32h743-exception-frame")]
unsafe fn load_fp_sentinels() {
    // Enable CP10/CP11 before the first VFP instruction, as H743 firmware
    // does on hardware.
    write_volatile(SCB_CPACR, read_volatile(SCB_CPACR) | (0xF << 20));
    asm!("dsb", "isb", options(nostack, preserves_flags));
    let values = core::ptr::addr_of!(FP_SENTINELS).cast::<u32>();
    asm!(
        "vldmia r0!, {{s0-s15}}",
        "movw r1, #0",
        "movt r1, #0xA000",
        "vmsr fpscr, r1",
        inout("r0") values => _,
        out("r1") _,
        options(nostack)
    );
}

/// Copy the live caller-owned FP state after exception return to ordinary RAM.
#[cfg(feature = "stm32h743-exception-frame")]
unsafe fn capture_fp_sentinels() {
    let capture = core::ptr::addr_of_mut!(FP_CAPTURE).cast::<u32>();
    asm!(
        "vstmia r0!, {{s0-s15}}",
        "vmrs r1, fpscr",
        "str r1, [r0]",
        inout("r0") capture => _,
        out("r1") _,
        options(nostack)
    );
}

#[cfg(feature = "stm32h743-exception-frame")]
fn check_cortex_m7_exception_frame() -> bool {
    let vector_table = unsafe { core::ptr::addr_of_mut!(RAM_VECTOR_TABLE.0).cast::<u32>() };
    let record = core::ptr::addr_of_mut!(EXCEPTION_RECORD).cast::<u32>();
    let source_vtor = unsafe { read_volatile(SCB_VTOR) };
    let source_ccr = unsafe { read_volatile(SCB_CCR) };

    unsafe {
        write_volatile(NVIC_ICER0, EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT);
        write_volatile(NVIC_ICPR0, EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT);
        for index in 0..64 {
            write_volatile(vector_table.add(index), 0);
        }
        for index in 0..18 {
            write_volatile(record.add(index), 0);
        }
        for index in 0..6 {
            write_volatile(
                core::ptr::addr_of_mut!(FP_NEST_RECORD)
                    .cast::<u32>()
                    .add(index),
                0,
            );
        }
        let handler = (smoke_exception_handler as *const () as usize as u32) | 1;
        let nested_handler = (smoke_nested_exception_handler as *const () as usize as u32) | 1;
        write_volatile(vector_table.add(EXCEPTION_VECTOR_INDEX), handler);
        write_volatile(vector_table.add(NESTED_VECTOR_INDEX), nested_handler);
        write_volatile(NVIC_IPR0.add(EXCEPTION_IRQ as usize), 0x80);
        write_volatile(NVIC_IPR0.add(NESTED_IRQ as usize), 0x20);
        write_volatile(SCB_VTOR, vector_table as u32);
        write_volatile(NVIC_ISER0, EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT);
    }

    let vector_installed = unsafe {
        read_volatile(vector_table.add(EXCEPTION_VECTOR_INDEX))
            == ((smoke_exception_handler as *const () as usize as u32) | 1)
    };
    let vtor_installed = unsafe { read_volatile(SCB_VTOR) } == vector_table as u32;

    let msp_before: u32;
    unsafe { asm!("mrs {}, msp", out(reg) msp_before, options(nomem, nostack, preserves_flags)) };
    unsafe { pend_exception_irq_with_sentinels() };
    let msp_after: u32;
    unsafe { asm!("mrs {}, msp", out(reg) msp_after, options(nomem, nostack, preserves_flags)) };

    let read_record = |index: usize| unsafe { read_volatile(record.add(index)) };
    let frame_r0 = read_record(0);
    let frame_r1 = read_record(1);
    let frame_r2 = read_record(2);
    let frame_r3 = read_record(3);
    let frame_r12 = read_record(4);
    let frame_lr = read_record(5);
    let frame_pc = read_record(6);
    let frame_xpsr = read_record(7);
    let exc_return = read_record(8);
    let frame_sp = read_record(9);
    let ipsr = read_record(10);
    let active_during = read_record(11);
    let pending_during = read_record(12);
    let msp_before_irq = read_record(13);

    let returned = unsafe { read_volatile(NVIC_ISPR0) } & EXCEPTION_IRQ_BIT == 0
        && unsafe { read_volatile(NVIC_IABR0) } & EXCEPTION_IRQ_BIT == 0;
    let frame_ok = frame_r0 == 0x11
        && frame_r1 == 0x22
        && frame_r2 == 0x33
        && frame_r3 == 0x44
        && frame_r12 == 0x55
        && frame_lr & 1 != 0
        && (0x0800_0000..0x0810_0000).contains(&frame_pc)
        && frame_xpsr & 0x0100_0000 != 0
        && exc_return == 0xffff_fff9
        && frame_sp & 7 == 0
        && frame_sp == msp_before_irq.wrapping_sub(32)
        && (0x2000_0000..0x2002_0000).contains(&frame_sp)
        && msp_after == msp_before;
    let active_and_pending_ok = ipsr == 16 + EXCEPTION_IRQ
        && active_during & EXCEPTION_IRQ_BIT != 0
        && pending_during & EXCEPTION_IRQ_BIT == 0;

    // Reuse the installed vector for a second entry from Thread/PSP. The
    // helper keeps its own saved registers on MSP and uses a dedicated
    // doubleword-aligned PSP inside the imported H743 RAM window.
    unsafe {
        for index in 0..18 {
            write_volatile(record.add(index), 0);
        }
    }
    let msp_before_psp_irq: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_before_psp_irq,
            options(nomem, nostack, preserves_flags)
        )
    };
    unsafe { pend_exception_irq_with_psp_sentinels(0x2001_F000) };
    let msp_after_psp_irq: u32;
    let psp_after: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            "mrs {}, psp",
            out(reg) msp_after_psp_irq,
            out(reg) psp_after,
            options(nomem, nostack, preserves_flags)
        )
    };
    let psp_exc_return = read_record(8);
    let psp_frame_sp = read_record(9);
    let psp_ipsr = read_record(10);
    let psp_active_during = read_record(11);
    let psp_pending_during = read_record(12);
    let psp_before_irq = read_record(13);
    let psp_frame_ok = read_record(0) == 0x11
        && read_record(1) == 0x22
        && read_record(2) == 0x33
        && read_record(3) == 0x44
        && read_record(4) == 0x55
        && read_record(5) & 1 != 0
        && (0x0800_0000..0x0810_0000).contains(&read_record(6))
        && read_record(7) & 0x0100_0000 != 0
        && psp_exc_return == 0xffff_fffd
        && psp_frame_sp & 7 == 0
        && psp_frame_sp == psp_before_irq.wrapping_sub(32)
        && (0x2000_0000..0x2002_0000).contains(&psp_frame_sp);
    let psp_active_and_pending_ok = psp_ipsr == 16 + EXCEPTION_IRQ
        && psp_active_during & EXCEPTION_IRQ_BIT != 0
        && psp_pending_during & EXCEPTION_IRQ_BIT == 0;
    let psp_returned = unsafe { read_volatile(NVIC_ISPR0) } & EXCEPTION_IRQ_BIT == 0
        && unsafe { read_volatile(NVIC_IABR0) } & EXCEPTION_IRQ_BIT == 0;
    let psp_ok = psp_frame_ok
        && psp_active_and_pending_ok
        && psp_returned
        && psp_after == psp_before_irq
        && msp_after_psp_irq == msp_before_psp_irq;

    // A four-byte-aligned Thread PSP must receive one alignment word on
    // exception entry when CCR.STKALIGN is set. The stacked xPSR marks that
    // padding, and exception return restores the original odd-word PSP.
    unsafe {
        for index in 0..18 {
            write_volatile(record.add(index), 0);
        }
    }
    let msp_before_aligned_psp_irq: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_before_aligned_psp_irq,
            options(nomem, nostack, preserves_flags)
        )
    };
    unsafe { pend_exception_irq_with_psp_sentinels(0x2001_F004) };
    let msp_after_aligned_psp_irq: u32;
    let aligned_psp_after: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            "mrs {}, psp",
            out(reg) msp_after_aligned_psp_irq,
            out(reg) aligned_psp_after,
            options(nomem, nostack, preserves_flags)
        )
    };
    let aligned_psp_exc_return = read_record(8);
    let aligned_psp_frame_sp = read_record(9);
    let aligned_psp_ipsr = read_record(10);
    let aligned_psp_active_during = read_record(11);
    let aligned_psp_pending_during = read_record(12);
    let aligned_psp_before_irq = read_record(13);
    let aligned_psp_xpsr = read_record(7);
    let aligned_psp_frame_ok = read_record(0) == 0x11
        && read_record(1) == 0x22
        && read_record(2) == 0x33
        && read_record(3) == 0x44
        && read_record(4) == 0x55
        && read_record(5) & 1 != 0
        && (0x0800_0000..0x0810_0000).contains(&read_record(6))
        && aligned_psp_xpsr & 0x0100_0000 != 0
        && aligned_psp_xpsr & 0x0000_0200 != 0
        && aligned_psp_exc_return == 0xffff_fffd
        && aligned_psp_before_irq & 7 == 4
        && aligned_psp_frame_sp == aligned_psp_before_irq.wrapping_sub(36)
        && aligned_psp_frame_sp & 7 == 0
        && (0x2000_0000..0x2002_0000).contains(&aligned_psp_frame_sp);
    let aligned_psp_active_and_pending_ok = aligned_psp_ipsr == 16 + EXCEPTION_IRQ
        && aligned_psp_active_during & EXCEPTION_IRQ_BIT != 0
        && aligned_psp_pending_during & EXCEPTION_IRQ_BIT == 0;
    let aligned_psp_returned = unsafe { read_volatile(NVIC_ISPR0) } & EXCEPTION_IRQ_BIT == 0
        && unsafe { read_volatile(NVIC_IABR0) } & EXCEPTION_IRQ_BIT == 0;
    let aligned_psp_ok = aligned_psp_frame_ok
        && aligned_psp_active_and_pending_ok
        && aligned_psp_returned
        && aligned_psp_after == aligned_psp_before_irq
        && msp_after_aligned_psp_irq == msp_before_aligned_psp_irq;

    // With CCR.STKALIGN clear, exception entry must leave a four-byte-aligned
    // PSP as-is apart from the eight-word frame and must not set xPSR[9].
    unsafe {
        write_volatile(SCB_CCR, source_ccr & !SCB_CCR_STKALIGN);
        for index in 0..18 {
            write_volatile(record.add(index), 0);
        }
    }
    let ccr_stkalign_cleared = unsafe { read_volatile(SCB_CCR) } & SCB_CCR_STKALIGN == 0;
    let msp_before_unaligned_psp_irq: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_before_unaligned_psp_irq,
            options(nomem, nostack, preserves_flags)
        )
    };
    unsafe { pend_exception_irq_with_psp_sentinels(0x2001_F004) };
    let msp_after_unaligned_psp_irq: u32;
    let unaligned_psp_after: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            "mrs {}, psp",
            out(reg) msp_after_unaligned_psp_irq,
            out(reg) unaligned_psp_after,
            options(nomem, nostack, preserves_flags)
        )
    };
    let unaligned_psp_exc_return = read_record(8);
    let unaligned_psp_frame_sp = read_record(9);
    let unaligned_psp_ipsr = read_record(10);
    let unaligned_psp_active_during = read_record(11);
    let unaligned_psp_pending_during = read_record(12);
    let unaligned_psp_before_irq = read_record(13);
    let unaligned_psp_xpsr = read_record(7);
    let unaligned_psp_frame_ok = read_record(0) == 0x11
        && read_record(1) == 0x22
        && read_record(2) == 0x33
        && read_record(3) == 0x44
        && read_record(4) == 0x55
        && read_record(5) & 1 != 0
        && (0x0800_0000..0x0810_0000).contains(&read_record(6))
        && unaligned_psp_xpsr & 0x0100_0000 != 0
        && unaligned_psp_xpsr & 0x0000_0200 == 0
        && unaligned_psp_exc_return == 0xffff_fffd
        && unaligned_psp_before_irq & 7 == 4
        && unaligned_psp_frame_sp == unaligned_psp_before_irq.wrapping_sub(32)
        && unaligned_psp_frame_sp & 7 == 4
        && (0x2000_0000..0x2002_0000).contains(&unaligned_psp_frame_sp);
    let unaligned_psp_active_and_pending_ok = unaligned_psp_ipsr == 16 + EXCEPTION_IRQ
        && unaligned_psp_active_during & EXCEPTION_IRQ_BIT != 0
        && unaligned_psp_pending_during & EXCEPTION_IRQ_BIT == 0;
    let unaligned_psp_returned = unsafe { read_volatile(NVIC_ISPR0) } & EXCEPTION_IRQ_BIT == 0
        && unsafe { read_volatile(NVIC_IABR0) } & EXCEPTION_IRQ_BIT == 0;
    let unaligned_psp_ok = ccr_stkalign_cleared
        && unaligned_psp_frame_ok
        && unaligned_psp_active_and_pending_ok
        && unaligned_psp_returned
        && unaligned_psp_after == unaligned_psp_before_irq
        && msp_after_unaligned_psp_irq == msp_before_unaligned_psp_irq;
    unsafe { write_volatile(SCB_CCR, source_ccr) };
    let ccr_restored = unsafe { read_volatile(SCB_CCR) } == source_ccr;

    // First use of the FPU marks Thread mode's FP context active. The IRQ must
    // eagerly stack the extended frame with LSPEN clear, return with
    // EXC_RETURN[4] clear, and restore S0-S15/FPSCR before resuming this context.
    unsafe {
        write_volatile(FPU_FPCCR, 1 << 31); // ASPEN=1, LSPEN=0
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_TOUCH), 0);
        for index in 0..18 {
            write_volatile(record.add(index), 0);
        }
        load_fp_sentinels();
    }
    let msp_before_fp_irq: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_before_fp_irq,
            options(nomem, nostack, preserves_flags)
        )
    };
    unsafe { pend_exception_irq_with_sentinels() };
    let msp_after_fp_irq: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_after_fp_irq,
            options(nomem, nostack, preserves_flags)
        )
    };
    let control_after_fp: u32;
    unsafe {
        asm!(
            "mrs {}, control",
            out(reg) control_after_fp,
            options(nomem, nostack, preserves_flags)
        )
    };
    unsafe { capture_fp_sentinels() };
    let fp_exc_return = read_record(8);
    let fp_frame_sp = read_record(9);
    let fp_ipsr = read_record(10);
    let fp_active_during = read_record(11);
    let fp_pending_during = read_record(12);
    let fp_msp_before_irq = read_record(13);
    let fp_fpcar = unsafe { read_volatile(FPU_FPCAR) };
    let fp_lazy_entry_ccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>()) };
    let fp_lazy_after_ccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>().add(1)) };
    let fp_capture = core::ptr::addr_of_mut!(FP_CAPTURE).cast::<u32>();
    let fp_registers_restored =
        (0..16).all(|index| unsafe { read_volatile(fp_capture.add(index)) == FP_SENTINELS[index] });
    let fp_fpscr_restored = unsafe { read_volatile(fp_capture.add(16)) } == 0xA000_0000;
    let fp_frame_ok = read_record(0) == 0x11
        && read_record(1) == 0x22
        && read_record(2) == 0x33
        && read_record(3) == 0x44
        && read_record(4) == 0x55
        && read_record(5) & 1 != 0
        && (0x0800_0000..0x0810_0000).contains(&read_record(6))
        && read_record(7) & 0x0100_0000 != 0
        && read_record(14) == FP_SENTINELS[0]
        && read_record(15) == FP_SENTINELS[1]
        && read_record(16) == FP_SENTINELS[15]
        && read_record(17) == 0xA000_0000
        && fp_exc_return == 0xFFFF_FFE9
        && fp_frame_sp & 7 == 0
        && fp_frame_sp == fp_msp_before_irq.wrapping_sub(0x68)
        && (0x2000_0000..0x2002_0000).contains(&fp_frame_sp)
        && fp_fpcar == fp_frame_sp.wrapping_add(0x20)
        && fp_lazy_entry_ccr & 1 == 0
        && fp_lazy_after_ccr & 1 == 0
        && control_after_fp & 0x4 != 0
        && fp_registers_restored
        && fp_fpscr_restored;
    let fp_active_and_pending_ok = fp_ipsr == 16 + EXCEPTION_IRQ
        && fp_active_during & EXCEPTION_IRQ_BIT != 0
        && fp_pending_during & EXCEPTION_IRQ_BIT == 0;
    let fp_returned = unsafe { read_volatile(NVIC_ISPR0) } & EXCEPTION_IRQ_BIT == 0
        && unsafe { read_volatile(NVIC_IABR0) } & EXCEPTION_IRQ_BIT == 0;
    let fp_ok = fp_frame_ok
        && fp_active_and_pending_ok
        && fp_returned
        && msp_after_fp_irq == msp_before_fp_irq;

    // With LSPEN set, the same handler does not use the FPU. The frame still
    // reserves 0x68 bytes, but the S0-S15/FPSCR region must remain untouched;
    // LSPACT stays set inside the handler and clears when it returns.
    unsafe {
        write_volatile(FPU_FPCCR, (1 << 31) | (1 << 30));
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_TOUCH), 0);
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_FPCCR).cast::<u32>(), 0);
        write_volatile(
            core::ptr::addr_of_mut!(FP_LAZY_FPCCR).cast::<u32>().add(1),
            0,
        );
        load_fp_sentinels();
    }
    let msp_before_lazy_unused: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_before_lazy_unused,
            options(nomem, nostack, preserves_flags)
        )
    };
    // The naked IRQ-pender saves r4/r7 before recording the interrupted MSP.
    let lazy_unused_before_irq = msp_before_lazy_unused.wrapping_sub(8);
    let lazy_unused_frame_sp = lazy_unused_before_irq.wrapping_sub(0x68);
    unsafe {
        for index in 0..17u32 {
            write_volatile(
                (lazy_unused_frame_sp + 0x20 + 4 * index) as *mut u32,
                0xDEAD_0000 + index,
            );
        }
        for index in 0..18 {
            write_volatile(record.add(index), 0);
        }
    }
    unsafe { pend_exception_irq_with_sentinels() };
    let msp_after_lazy_unused: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_after_lazy_unused,
            options(nomem, nostack, preserves_flags)
        )
    };
    let lazy_unused_pre_fpccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>()) };
    let lazy_unused_post_fpccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>().add(1)) };
    let lazy_unused_fpccr_after_return = unsafe { read_volatile(FPU_FPCCR) };
    unsafe { capture_fp_sentinels() };
    let lazy_unused_capture = core::ptr::addr_of_mut!(FP_CAPTURE).cast::<u32>();
    let lazy_unused_regs_ok = (0..16).all(|index| unsafe {
        read_volatile(lazy_unused_capture.add(index)) == FP_SENTINELS[index]
    }) && unsafe { read_volatile(lazy_unused_capture.add(16)) }
        == 0xA000_0000;
    let lazy_unused_area_untouched = (0..17u32).all(|index| unsafe {
        read_volatile((lazy_unused_frame_sp + 0x20 + 4 * index) as *const u32)
            == 0xDEAD_0000 + index
    });
    let lazy_unused_ok = read_record(8) == 0xFFFF_FFE9
        && read_record(9) == lazy_unused_frame_sp
        && read_record(13) == lazy_unused_before_irq
        && lazy_unused_pre_fpccr & 1 != 0
        && lazy_unused_post_fpccr & 1 != 0
        && lazy_unused_fpccr_after_return & 1 == 0
        && lazy_unused_area_untouched
        && lazy_unused_regs_ok
        && msp_after_lazy_unused == msp_before_lazy_unused;

    // When the handler executes its first FP instruction, the model must save
    // the interrupted context into the reserved area first, then clear LSPACT.
    unsafe {
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_TOUCH), 1);
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_FPCCR).cast::<u32>(), 0);
        write_volatile(
            core::ptr::addr_of_mut!(FP_LAZY_FPCCR).cast::<u32>().add(1),
            0,
        );
        load_fp_sentinels();
    }
    let msp_before_lazy_used: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_before_lazy_used,
            options(nomem, nostack, preserves_flags)
        )
    };
    // Keep this aligned with the IRQ-pender's two saved registers above.
    let lazy_used_before_irq = msp_before_lazy_used.wrapping_sub(8);
    let lazy_used_frame_sp = lazy_used_before_irq.wrapping_sub(0x68);
    unsafe {
        for index in 0..17u32 {
            write_volatile(
                (lazy_used_frame_sp + 0x20 + 4 * index) as *mut u32,
                0xBEEF_0000 + index,
            );
        }
        for index in 0..18 {
            write_volatile(record.add(index), 0);
        }
    }
    unsafe { pend_exception_irq_with_sentinels() };
    let msp_after_lazy_used: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_after_lazy_used,
            options(nomem, nostack, preserves_flags)
        )
    };
    let lazy_used_pre_fpccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>()) };
    let lazy_used_post_fpccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>().add(1)) };
    let lazy_used_fpccr_after_return = unsafe { read_volatile(FPU_FPCCR) };
    let lazy_used_saved =
        (0..16u32).all(|index| unsafe {
            read_volatile((lazy_used_frame_sp + 0x20 + 4 * index) as *const u32)
                == FP_SENTINELS[index as usize]
        }) && unsafe { read_volatile((lazy_used_frame_sp + 0x60) as *const u32) == 0xA000_0000 };
    unsafe { capture_fp_sentinels() };
    let lazy_used_capture = core::ptr::addr_of_mut!(FP_CAPTURE).cast::<u32>();
    let lazy_used_regs_ok = (0..16)
        .all(|index| unsafe { read_volatile(lazy_used_capture.add(index)) == FP_SENTINELS[index] })
        && unsafe { read_volatile(lazy_used_capture.add(16)) } == 0xA000_0000;
    let lazy_used_ok = read_record(8) == 0xFFFF_FFE9
        && read_record(9) == lazy_used_frame_sp
        && read_record(13) == lazy_used_before_irq
        && lazy_used_pre_fpccr & 1 != 0
        && lazy_used_post_fpccr & 1 == 0
        && lazy_used_fpccr_after_return & 1 == 0
        && lazy_used_saved
        && lazy_used_regs_ok
        && msp_after_lazy_used == msp_before_lazy_used;

    // A higher-priority IRQ nests while IRQ 5 owns an outstanding lazy save.
    // IRQ 6 must see the same FPCAR/LSPACT and return without forcing the save;
    // IRQ 5's first FP instruction then performs the deferred context store.
    unsafe {
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_NEST), 1);
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_NEST_TOUCH), 0);
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_TOUCH), 1);
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_FPCCR).cast::<u32>(), 0);
        write_volatile(
            core::ptr::addr_of_mut!(FP_LAZY_FPCCR).cast::<u32>().add(1),
            0,
        );
        for index in 0..6 {
            write_volatile(
                core::ptr::addr_of_mut!(FP_NEST_RECORD)
                    .cast::<u32>()
                    .add(index),
                0,
            );
        }
        load_fp_sentinels();
    }
    let msp_before_nested_lazy: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_before_nested_lazy,
            options(nomem, nostack, preserves_flags)
        )
    };
    let nested_lazy_before_irq = msp_before_nested_lazy.wrapping_sub(8);
    let nested_lazy_frame_sp = nested_lazy_before_irq.wrapping_sub(0x68);
    unsafe {
        for index in 0..17u32 {
            write_volatile(
                (nested_lazy_frame_sp + 0x20 + 4 * index) as *mut u32,
                0xCAFE_0000 + index,
            );
        }
        for index in 0..18 {
            write_volatile(record.add(index), 0);
        }
        pend_exception_irq_with_sentinels();
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_NEST), 0);
    }
    let msp_after_nested_lazy: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_after_nested_lazy,
            options(nomem, nostack, preserves_flags)
        )
    };
    let nested_record = core::ptr::addr_of!(FP_NEST_RECORD).cast::<u32>();
    let nested_fpccr = unsafe { read_volatile(nested_record.add(0)) };
    let nested_fpcar = unsafe { read_volatile(nested_record.add(1)) };
    let nested_ipsr = unsafe { read_volatile(nested_record.add(2)) };
    let nested_active = unsafe { read_volatile(nested_record.add(3)) };
    let nested_exc_return = unsafe { read_volatile(nested_record.add(4)) };
    let nested_lazy_pre_fpccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>()) };
    let nested_lazy_post_fpccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>().add(1)) };
    let nested_lazy_fpccr_after_return = unsafe { read_volatile(FPU_FPCCR) };
    let nested_lazy_saved =
        (0..16u32).all(|index| unsafe {
            read_volatile((nested_lazy_frame_sp + 0x20 + 4 * index) as *const u32)
                == FP_SENTINELS[index as usize]
        }) && unsafe { read_volatile((nested_lazy_frame_sp + 0x60) as *const u32) == 0xA000_0000 };
    unsafe { capture_fp_sentinels() };
    let nested_lazy_capture = core::ptr::addr_of_mut!(FP_CAPTURE).cast::<u32>();
    let nested_lazy_regs_ok = (0..16).all(|index| unsafe {
        read_volatile(nested_lazy_capture.add(index)) == FP_SENTINELS[index]
    }) && unsafe { read_volatile(nested_lazy_capture.add(16)) }
        == 0xA000_0000;
    let nested_lazy_ok = read_record(8) == 0xFFFF_FFE9
        && read_record(9) == nested_lazy_frame_sp
        && read_record(13) == nested_lazy_before_irq
        && nested_fpccr & 1 != 0
        && nested_fpcar == nested_lazy_frame_sp.wrapping_add(0x20)
        && nested_ipsr == 16 + NESTED_IRQ
        && nested_active & (EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT)
            == EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT
        && nested_exc_return == 0xFFFF_FFF1
        && unsafe { read_volatile(nested_record.add(5)) } & 1 != 0
        && nested_lazy_pre_fpccr & 1 != 0
        && nested_lazy_post_fpccr & 1 == 0
        && nested_lazy_fpccr_after_return & 1 == 0
        && nested_lazy_saved
        && nested_lazy_regs_ok
        && unsafe { read_volatile(NVIC_IABR0) } & (EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT) == 0
        && unsafe { read_volatile(NVIC_ISPR0) } & (EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT) == 0
        && msp_after_nested_lazy == msp_before_nested_lazy;

    // This time the nested IRQ's first FP instruction completes the outer
    // Thread-mode lazy save. IRQ 5 itself does not touch the FPU; returning
    // from both handlers must still restore the interrupted Thread context.
    unsafe {
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_NEST), 1);
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_NEST_TOUCH), 1);
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_TOUCH), 0);
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_FPCCR).cast::<u32>(), 0);
        write_volatile(
            core::ptr::addr_of_mut!(FP_LAZY_FPCCR).cast::<u32>().add(1),
            0,
        );
        for index in 0..6 {
            write_volatile(
                core::ptr::addr_of_mut!(FP_NEST_RECORD)
                    .cast::<u32>()
                    .add(index),
                0,
            );
        }
        load_fp_sentinels();
    }
    let msp_before_nested_touch: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_before_nested_touch,
            options(nomem, nostack, preserves_flags)
        )
    };
    let nested_touch_before_irq = msp_before_nested_touch.wrapping_sub(8);
    let nested_touch_frame_sp = nested_touch_before_irq.wrapping_sub(0x68);
    unsafe {
        for index in 0..17u32 {
            write_volatile(
                (nested_touch_frame_sp + 0x20 + 4 * index) as *mut u32,
                0xFACE_0000 + index,
            );
        }
        for index in 0..18 {
            write_volatile(record.add(index), 0);
        }
        pend_exception_irq_with_sentinels();
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_NEST), 0);
        write_volatile(core::ptr::addr_of_mut!(FP_LAZY_NEST_TOUCH), 0);
    }
    let msp_after_nested_touch: u32;
    unsafe {
        asm!(
            "mrs {}, msp",
            out(reg) msp_after_nested_touch,
            options(nomem, nostack, preserves_flags)
        )
    };
    let nested_touch_fpccr = unsafe { read_volatile(nested_record.add(0)) };
    let nested_touch_fpcar = unsafe { read_volatile(nested_record.add(1)) };
    let nested_touch_ipsr = unsafe { read_volatile(nested_record.add(2)) };
    let nested_touch_active = unsafe { read_volatile(nested_record.add(3)) };
    let nested_touch_exc_return = unsafe { read_volatile(nested_record.add(4)) };
    let nested_touch_post_fpccr = unsafe { read_volatile(nested_record.add(5)) };
    let nested_touch_outer_pre_fpccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>()) };
    let nested_touch_outer_post_fpccr =
        unsafe { read_volatile(core::ptr::addr_of!(FP_LAZY_FPCCR).cast::<u32>().add(1)) };
    let nested_touch_fpccr_after_return = unsafe { read_volatile(FPU_FPCCR) };
    let nested_touch_saved =
        (0..16u32).all(|index| unsafe {
            read_volatile((nested_touch_frame_sp + 0x20 + 4 * index) as *const u32)
                == FP_SENTINELS[index as usize]
        }) && unsafe { read_volatile((nested_touch_frame_sp + 0x60) as *const u32) == 0xA000_0000 };
    unsafe { capture_fp_sentinels() };
    let nested_touch_capture = core::ptr::addr_of_mut!(FP_CAPTURE).cast::<u32>();
    let nested_touch_regs_ok = (0..16).all(|index| unsafe {
        read_volatile(nested_touch_capture.add(index)) == FP_SENTINELS[index]
    }) && unsafe { read_volatile(nested_touch_capture.add(16)) }
        == 0xA000_0000;
    let control_after_nested_touch: u32;
    unsafe {
        asm!(
            "mrs {}, control",
            out(reg) control_after_nested_touch,
            options(nomem, nostack, preserves_flags)
        )
    };
    let nested_touch_ok = read_record(8) == 0xFFFF_FFE9
        && read_record(9) == nested_touch_frame_sp
        && read_record(13) == nested_touch_before_irq
        && nested_touch_fpccr & 1 != 0
        && nested_touch_fpcar == nested_touch_frame_sp.wrapping_add(0x20)
        && nested_touch_ipsr == 16 + NESTED_IRQ
        && nested_touch_active & (EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT)
            == EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT
        && nested_touch_exc_return == 0xFFFF_FFF1
        && nested_touch_post_fpccr & 1 == 0
        && nested_touch_outer_pre_fpccr & 1 != 0
        && nested_touch_outer_post_fpccr & 1 == 0
        && nested_touch_fpccr_after_return & 1 == 0
        && control_after_nested_touch & 0x4 != 0
        && nested_touch_saved
        && nested_touch_regs_ok
        && unsafe { read_volatile(NVIC_IABR0) } & (EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT) == 0
        && unsafe { read_volatile(NVIC_ISPR0) } & (EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT) == 0
        && msp_after_nested_touch == msp_before_nested_touch;

    unsafe {
        write_volatile(NVIC_ICER0, EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT);
        write_volatile(NVIC_ICPR0, EXCEPTION_IRQ_BIT | NESTED_IRQ_BIT);
        write_volatile(SCB_VTOR, source_vtor);
    }
    let vtor_restored = unsafe { read_volatile(SCB_VTOR) } == source_vtor;
    let ok = vector_installed
        && vtor_installed
        && frame_ok
        && active_and_pending_ok
        && returned
        && vtor_restored
        && psp_ok
        && aligned_psp_ok
        && unaligned_psp_ok
        && fp_ok
        && lazy_unused_ok
        && lazy_used_ok
        && nested_lazy_ok
        && nested_touch_ok
        && ccr_restored;

    send(b"cortex-m7 irq ipsr=");
    send_hex32(ipsr);
    send(b" active=");
    send_hex32(active_during);
    send(b" pending=");
    send_hex32(pending_during);
    send(b"\r\ncortex-m7 frame=");
    send(if frame_ok { b"1" } else { b"0" });
    send(b" regs=");
    send_hex32(frame_r0);
    send(b",");
    send_hex32(frame_r1);
    send(b",");
    send_hex32(frame_r2);
    send(b",");
    send_hex32(frame_r3);
    send(b",");
    send_hex32(frame_r12);
    send(b"\r\ncortex-m7 return=");
    send(if returned { b"1" } else { b"0" });
    send(b" stack=");
    send(if msp_after == msp_before { b"1" } else { b"0" });
    send(b" vtor=");
    send(if vtor_installed && vtor_restored {
        b"1"
    } else {
        b"0"
    });
    send(b"\r\ncortex-m7 psp exception=");
    send(if psp_frame_ok && psp_active_and_pending_ok {
        b"1"
    } else {
        b"0"
    });
    send(b" stack=");
    send(if psp_after == psp_before_irq {
        b"1"
    } else {
        b"0"
    });
    send(b" msp=");
    send(if msp_after_psp_irq == msp_before_psp_irq {
        b"1"
    } else {
        b"0"
    });
    send(b" return=");
    send(if psp_returned { b"1" } else { b"0" });
    send(b" exc_return=");
    send_hex32(psp_exc_return);
    send(b"\r\ncortex-m7 psp align exception=");
    send(
        if aligned_psp_frame_ok && aligned_psp_active_and_pending_ok {
            b"1"
        } else {
            b"0"
        },
    );
    send(b" frame=");
    send(if aligned_psp_frame_ok { b"1" } else { b"0" });
    send(b" stack=");
    send(if aligned_psp_after == aligned_psp_before_irq {
        b"1"
    } else {
        b"0"
    });
    send(b" msp=");
    send(if msp_after_aligned_psp_irq == msp_before_aligned_psp_irq {
        b"1"
    } else {
        b"0"
    });
    send(b" return=");
    send(if aligned_psp_returned { b"1" } else { b"0" });
    send(b" xpsr=");
    send_hex32(aligned_psp_xpsr);
    send(b" sp=");
    send_hex32(aligned_psp_frame_sp);
    send(b"\r\n");
    send(b"cortex-m7 psp noalign exception=");
    send(
        if unaligned_psp_frame_ok && unaligned_psp_active_and_pending_ok {
            b"1"
        } else {
            b"0"
        },
    );
    send(b" frame=");
    send(if unaligned_psp_frame_ok { b"1" } else { b"0" });
    send(b" stack=");
    send(if unaligned_psp_after == unaligned_psp_before_irq {
        b"1"
    } else {
        b"0"
    });
    send(b" msp=");
    send(
        if msp_after_unaligned_psp_irq == msp_before_unaligned_psp_irq {
            b"1"
        } else {
            b"0"
        },
    );
    send(b" return=");
    send(if unaligned_psp_returned { b"1" } else { b"0" });
    send(b" ccr=");
    send(if ccr_stkalign_cleared && ccr_restored {
        b"1"
    } else {
        b"0"
    });
    send(b" xpsr=");
    send_hex32(unaligned_psp_xpsr);
    send(b" sp=");
    send_hex32(unaligned_psp_frame_sp);
    send(b"\r\n");
    send(b"cortex-m7 fp exception=");
    send(if fp_frame_ok && fp_active_and_pending_ok {
        b"1"
    } else {
        b"0"
    });
    send(b" frame=");
    send(if fp_frame_ok { b"1" } else { b"0" });
    send(b" stack=");
    send(if msp_after_fp_irq == msp_before_fp_irq {
        b"1"
    } else {
        b"0"
    });
    send(b" return=");
    send(if fp_returned { b"1" } else { b"0" });
    send(b" regs=");
    send(if fp_registers_restored { b"1" } else { b"0" });
    send(b" fpscr=");
    send(if fp_fpscr_restored { b"1" } else { b"0" });
    send(b" fpcar=");
    send(if fp_fpcar == fp_frame_sp.wrapping_add(0x20) {
        b"1"
    } else {
        b"0"
    });
    send(b" exc_return=");
    send_hex32(fp_exc_return);
    send(b" sp=");
    send_hex32(fp_frame_sp);
    send(b"\r\ncortex-m7 fp lazy unused=");
    send(if lazy_unused_ok { b"1" } else { b"0" });
    send(b" lspact=");
    send_hex32(lazy_unused_pre_fpccr & 1);
    send(b",");
    send_hex32(lazy_unused_post_fpccr & 1);
    send(b"->");
    send_hex32(lazy_unused_fpccr_after_return & 1);
    send(b" untouched=");
    send(if lazy_unused_area_untouched {
        b"1"
    } else {
        b"0"
    });
    send(b" regs=");
    send(if lazy_unused_regs_ok { b"1" } else { b"0" });
    send(b"\r\ncortex-m7 fp lazy used=");
    send(if lazy_used_ok { b"1" } else { b"0" });
    send(b" lspact=");
    send_hex32(lazy_used_pre_fpccr & 1);
    send(b",");
    send_hex32(lazy_used_post_fpccr & 1);
    send(b"->");
    send_hex32(lazy_used_fpccr_after_return & 1);
    send(b" saved=");
    send(if lazy_used_saved { b"1" } else { b"0" });
    send(b" regs=");
    send(if lazy_used_regs_ok { b"1" } else { b"0" });
    send(b"\r\ncortex-m7 fp lazy nested=");
    send(if nested_lazy_ok { b"1" } else { b"0" });
    send(b" lspact=");
    send_hex32(nested_fpccr & 1);
    send(b"->");
    send_hex32(nested_lazy_pre_fpccr & 1);
    send(b",");
    send_hex32(nested_lazy_post_fpccr & 1);
    send(b" saved=");
    send(if nested_lazy_saved { b"1" } else { b"0" });
    send(b" regs=");
    send(if nested_lazy_regs_ok { b"1" } else { b"0" });
    send(b" irq=");
    send_hex32(nested_ipsr);
    send(b" active=");
    send_hex32(nested_active);
    send(b" exc_return=");
    send_hex32(nested_exc_return);
    send(b"\r\n");
    send(b"cortex-m7 fp lazy nested-touch=");
    send(if nested_touch_ok { b"1" } else { b"0" });
    send(b" lspact=");
    send_hex32(nested_touch_fpccr & 1);
    send(b"->");
    send_hex32(nested_touch_post_fpccr & 1);
    send(b" saved=");
    send(if nested_touch_saved { b"1" } else { b"0" });
    send(b" regs=");
    send(if nested_touch_regs_ok { b"1" } else { b"0" });
    send(b" active=");
    send_hex32(nested_touch_active);
    send(b"\r\n");

    // Privileged-only CP10/CP11 access must still permit FP in privileged
    // Thread mode. Verify a real register round-trip before testing the same
    // CPACR setting from unprivileged Thread mode below.
    unsafe {
        write_volatile(SCB_CPACR, CPACR_PRIVILEGED_ONLY);
        write_volatile(SCB_CFSR, CFSR_UFSR_NOCP);
        asm!("dsb", "isb", options(nostack, preserves_flags));
    }
    let privileged_fp_roundtrip = unsafe { privileged_only_fp_roundtrip() };
    let privileged_nocp = unsafe { read_volatile(SCB_CFSR) } & CFSR_UFSR_NOCP;
    let privileged_fp_ok = privileged_fp_roundtrip == 0xA5 && privileged_nocp == 0;
    send(b"cortex-m7 cpacr privileged fp=");
    send(if privileged_fp_ok { b"1" } else { b"0" });
    send(b" nocp=");
    send_hex32(privileged_nocp);
    send(b" result=");
    send_hex32(privileged_fp_roundtrip);
    send(b"\r\n");

    // CPACR=00 denies a real VFP instruction even in privileged Thread mode.
    // Then CPACR privileged-only must fault the same instruction from
    // unprivileged Thread mode. The fault handler records each stacked frame
    // and skips the denied instruction so the smoke can continue.
    let nocp_vector = (nocp_fault_handler as *const () as usize as u32) | 1;
    unsafe {
        let vector_table = core::ptr::addr_of_mut!(RAM_VECTOR_TABLE.0).cast::<u32>();
        let record = core::ptr::addr_of_mut!(CPACR_FAULT_RECORD).cast::<u32>();
        write_volatile(vector_table.add(3), nocp_vector);
        write_volatile(vector_table.add(6), nocp_vector);
        write_volatile(
            vector_table.add(SVC_VECTOR_INDEX),
            (cpacr_svc_handler as *const () as usize as u32) | 1,
        );
        write_volatile(SCB_VTOR, vector_table as u32);
        for index in 0..5 {
            write_volatile(record.add(index), 0);
        }
        let handler_record = core::ptr::addr_of_mut!(CPACR_HANDLER_RECORD).cast::<u32>();
        write_volatile(handler_record, 0);
        write_volatile(handler_record.add(1), 0);
        write_volatile(SCB_CFSR, CFSR_UFSR_NOCP); // clear a stale NOCP latch
        write_volatile(SCB_SHCSR, read_volatile(SCB_SHCSR) | SHCSR_USGFAULTENA);
        write_volatile(SCB_CPACR, 0);
        asm!("dsb", "isb", options(nostack, preserves_flags));
        attempt_privileged_fp();
    }
    let denied_record = core::ptr::addr_of!(CPACR_FAULT_RECORD).cast::<u32>();
    let denied_pc = unsafe { read_volatile(denied_record) };
    let denied_cfsr = unsafe { read_volatile(denied_record.add(1)) };
    let denied_ipsr = unsafe { read_volatile(denied_record.add(2)) };
    let privileged_denied_ok = denied_pc >= 0x0800_0000
        && denied_pc < 0x0810_0000
        && denied_cfsr & CFSR_UFSR_NOCP != 0
        && denied_ipsr == 6;
    send(b"cortex-m7 cpacr privileged denied=");
    send(if privileged_denied_ok { b"1" } else { b"0" });
    send(b" ipsr=");
    send_hex32(denied_ipsr);
    send(b" cfsr=");
    send_hex32(denied_cfsr & CFSR_UFSR_NOCP);
    send(b"\r\n");

    // The architectural 0b10 access encoding is reserved and unpredictable.
    // This engine chooses a deterministic safe policy: deny the access and
    // raise NOCP. Keep this assertion scoped to that engine policy.
    unsafe {
        let record = core::ptr::addr_of_mut!(CPACR_FAULT_RECORD).cast::<u32>();
        for index in 0..5 {
            write_volatile(record.add(index), 0);
        }
        write_volatile(SCB_CFSR, CFSR_UFSR_NOCP);
        write_volatile(SCB_CPACR, CPACR_RESERVED_ACCESS);
        asm!("dsb", "isb", options(nostack, preserves_flags));
        attempt_privileged_fp();
    }
    let reserved_record = core::ptr::addr_of!(CPACR_FAULT_RECORD).cast::<u32>();
    let reserved_pc = unsafe { read_volatile(reserved_record) };
    let reserved_cfsr = unsafe { read_volatile(reserved_record.add(1)) };
    let reserved_ipsr = unsafe { read_volatile(reserved_record.add(2)) };
    let reserved_readback = unsafe { read_volatile(SCB_CPACR) } & CPACR_CP10_CP11;
    let reserved_denied_ok = reserved_pc >= 0x0800_0000
        && reserved_pc < 0x0810_0000
        && reserved_cfsr & CFSR_UFSR_NOCP != 0
        && reserved_ipsr == 6
        && reserved_readback == CPACR_RESERVED_ACCESS;
    send(b"cortex-m7 cpacr reserved policy-deny=");
    send(if reserved_denied_ok { b"1" } else { b"0" });
    send(b" readback=");
    send_hex32(reserved_readback);
    send(b" ipsr=");
    send_hex32(reserved_ipsr);
    send(b" cfsr=");
    send_hex32(reserved_cfsr & CFSR_UFSR_NOCP);
    send(b"\r\n");

    // With UsageFault disabled, the denied FP operation must escalate to
    // HardFault, retain CFSR.NOCP, and set HFSR.FORCED. The same small handler
    // captures the stacked instruction address before skipping it.
    unsafe {
        let record = core::ptr::addr_of_mut!(CPACR_FAULT_RECORD).cast::<u32>();
        for index in 0..5 {
            write_volatile(record.add(index), 0);
        }
        write_volatile(SCB_HFSR, HFSR_FORCED);
        write_volatile(SCB_CFSR, CFSR_UFSR_NOCP);
        write_volatile(SCB_SHCSR, read_volatile(SCB_SHCSR) & !SHCSR_USGFAULTENA);
        write_volatile(SCB_CPACR, 0);
        asm!("dsb", "isb", options(nostack, preserves_flags));
        attempt_privileged_fp();
    }
    let escalated_record = core::ptr::addr_of!(CPACR_FAULT_RECORD).cast::<u32>();
    let escalated_pc = unsafe { read_volatile(escalated_record) };
    let escalated_cfsr = unsafe { read_volatile(escalated_record.add(1)) };
    let escalated_ipsr = unsafe { read_volatile(escalated_record.add(2)) };
    let escalated_hfsr = unsafe { read_volatile(escalated_record.add(4)) };
    let nocp_escalated_ok = escalated_pc >= 0x0800_0000
        && escalated_pc < 0x0810_0000
        && escalated_cfsr & CFSR_UFSR_NOCP != 0
        && escalated_ipsr == 3
        && escalated_hfsr & HFSR_FORCED != 0;
    send(b"cortex-m7 cpacr disabled usagefault hardfault=");
    send(if nocp_escalated_ok { b"1" } else { b"0" });
    send(b" ipsr=");
    send_hex32(escalated_ipsr);
    send(b" cfsr=");
    send_hex32(escalated_cfsr & CFSR_UFSR_NOCP);
    send(b" hfsr=");
    send_hex32(escalated_hfsr & HFSR_FORCED);
    send(b"\r\n");

    unsafe {
        write_volatile(SCB_SHCSR, read_volatile(SCB_SHCSR) | SHCSR_USGFAULTENA);
    }

    // UsageFault is enabled here, but its priority is lower than IRQ 5's.
    // A denied VFP operation inside IRQ 5 must still escalate to HardFault.
    let priority_vector = (cpacr_priority_irq_handler as *const () as usize as u32) | 1;
    let prior_irq_vector = unsafe { read_volatile(vector_table.add(EXCEPTION_VECTOR_INDEX)) };
    let prior_shpr1 = unsafe { read_volatile(SCB_SHPR1) };
    let prior_shcsr = unsafe { read_volatile(SCB_SHCSR) };
    let priority_irq_record = core::ptr::addr_of_mut!(CPACR_PRIORITY_IRQ_RECORD).cast::<u32>();
    let cpacr_fault_record = core::ptr::addr_of_mut!(CPACR_FAULT_RECORD).cast::<u32>();
    unsafe {
        write_volatile(NVIC_ICER0, EXCEPTION_IRQ_BIT);
        write_volatile(NVIC_ICPR0, EXCEPTION_IRQ_BIT);
        write_volatile(vector_table.add(EXCEPTION_VECTOR_INDEX), priority_vector);
        for index in 0..2 {
            write_volatile(priority_irq_record.add(index), 0);
        }
        for index in 0..5 {
            write_volatile(cpacr_fault_record.add(index), 0);
        }
        // SHPR1's UsageFault priority byte is bits 16..23. 0xF0 cannot
        // preempt IRQ 5 at priority 0x80.
        write_volatile(SCB_SHPR1, (prior_shpr1 & !0x00FF_0000) | 0x00F0_0000);
        write_volatile(SCB_CFSR, CFSR_UFSR_NOCP);
        write_volatile(SCB_HFSR, HFSR_FORCED);
        write_volatile(SCB_CPACR, 0);
        asm!("dsb", "isb", options(nostack, preserves_flags));
        write_volatile(NVIC_ISER0, EXCEPTION_IRQ_BIT);
        write_volatile(NVIC_ISPR0, EXCEPTION_IRQ_BIT);
    }
    let priority_irq_ipsr = unsafe { read_volatile(priority_irq_record) };
    let priority_irq_started = unsafe { read_volatile(priority_irq_record.add(1)) } == 1;
    let priority_fault_pc = unsafe { read_volatile(cpacr_fault_record) };
    let priority_fault_cfsr = unsafe { read_volatile(cpacr_fault_record.add(1)) };
    let priority_fault_ipsr = unsafe { read_volatile(cpacr_fault_record.add(2)) };
    let priority_fault_hfsr = unsafe { read_volatile(cpacr_fault_record.add(4)) };
    let priority_fault_shcsr = unsafe { read_volatile(SCB_SHCSR) };
    let priority_irq_active_after = unsafe { read_volatile(NVIC_IABR0) } & EXCEPTION_IRQ_BIT;
    let priority_irq_pending_after = unsafe { read_volatile(NVIC_ISPR0) } & EXCEPTION_IRQ_BIT;
    let priority_escalated_ok = priority_irq_started
        && priority_irq_ipsr == 16 + EXCEPTION_IRQ
        && priority_fault_pc >= 0x0800_0000
        && priority_fault_pc < 0x0810_0000
        && priority_fault_cfsr & CFSR_UFSR_NOCP != 0
        && priority_fault_ipsr == 3
        && priority_fault_hfsr & HFSR_FORCED != 0
        && priority_fault_shcsr & SHCSR_USGFAULTENA != 0
        && priority_irq_active_after == 0
        && priority_irq_pending_after == 0;
    unsafe {
        write_volatile(vector_table.add(EXCEPTION_VECTOR_INDEX), prior_irq_vector);
        write_volatile(SCB_SHPR1, prior_shpr1);
        write_volatile(NVIC_ICPR0, EXCEPTION_IRQ_BIT);
        write_volatile(SCB_CFSR, CFSR_UFSR_NOCP);
        write_volatile(SCB_HFSR, HFSR_FORCED);
        write_volatile(SCB_CPACR, CPACR_PRIVILEGED_ONLY);
        let shcsr = read_volatile(SCB_SHCSR);
        write_volatile(
            SCB_SHCSR,
            if prior_shcsr & SHCSR_USGFAULTENA != 0 {
                shcsr | SHCSR_USGFAULTENA
            } else {
                shcsr & !SHCSR_USGFAULTENA
            },
        );
    }
    send(b"cortex-m7 cpacr priority escalation=");
    send(if priority_escalated_ok { b"1" } else { b"0" });
    send(b" active_irq=");
    send_hex32(priority_irq_ipsr);
    send(b" ipsr=");
    send_hex32(priority_fault_ipsr);
    send(b" cfsr=");
    send_hex32(priority_fault_cfsr & CFSR_UFSR_NOCP);
    send(b" hfsr=");
    send_hex32(priority_fault_hfsr & HFSR_FORCED);
    send(b"\r\n");

    unsafe {
        let record = core::ptr::addr_of_mut!(CPACR_FAULT_RECORD).cast::<u32>();
        for index in 0..5 {
            write_volatile(record.add(index), 0);
        }
        write_volatile(SCB_CFSR, CFSR_UFSR_NOCP);
        write_volatile(SCB_CPACR, CPACR_PRIVILEGED_ONLY);
        asm!("dsb", "isb", options(nostack, preserves_flags));
        attempt_unprivileged_fp();
    }
    let nocp_record = core::ptr::addr_of!(CPACR_FAULT_RECORD).cast::<u32>();
    let nocp_pc = unsafe { read_volatile(nocp_record) };
    let nocp_cfsr = unsafe { read_volatile(nocp_record.add(1)) };
    let nocp_ipsr = unsafe { read_volatile(nocp_record.add(2)) };
    let nocp_exc_return = unsafe { read_volatile(nocp_record.add(3)) };
    let nocp_ok = nocp_pc >= 0x0800_0000
        && nocp_pc < 0x0810_0000
        && nocp_cfsr & CFSR_UFSR_NOCP != 0
        && nocp_ipsr == 6
        && nocp_exc_return & 0x10 == 0;
    send(b"cortex-m7 cpacr nocp fault=");
    send(if nocp_ok { b"1" } else { b"0" });
    send(b" ipsr=");
    send_hex32(nocp_ipsr);
    send(b" cfsr=");
    send_hex32(nocp_cfsr & CFSR_UFSR_NOCP);
    send(b" extended=");
    send(if nocp_exc_return & 0x10 == 0 {
        b"1\r\n"
    } else {
        b"0\r\n"
    });

    // Full CP10/CP11 access must also permit an actual FP operation in
    // unprivileged Thread mode. Clear the previous NOCP latch first, then
    // round-trip a known integer through S31 so a skipped instruction cannot
    // masquerade as a successful access.
    unsafe {
        write_volatile(SCB_CPACR, CPACR_CP10_CP11);
        write_volatile(SCB_CFSR, CFSR_UFSR_NOCP);
        asm!("dsb", "isb", options(nostack, preserves_flags));
    }
    let fp_roundtrip = unsafe { unprivileged_full_access_fp_roundtrip() };
    let full_access_nocp = unsafe { read_volatile(SCB_CFSR) } & CFSR_UFSR_NOCP;
    let full_access_ok = fp_roundtrip == 0x5A && full_access_nocp == 0;
    send(b"cortex-m7 cpacr unprivileged fp=");
    send(if full_access_ok { b"1" } else { b"0" });
    send(b" nocp=");
    send_hex32(full_access_nocp);
    send(b" result=");
    send_hex32(fp_roundtrip);
    send(b"\r\n");

    // The unprivileged Thread issues SVC after demonstrating full access.
    // Its handler changes CPACR back to privileged-only and performs a real
    // VFP round trip, proving Handler mode retains privileged access.
    let thread_unprivileged = unsafe { attempt_unprivileged_svc() };
    let handler_record = core::ptr::addr_of!(CPACR_HANDLER_RECORD).cast::<u32>();
    let handler_fp_result = unsafe { read_volatile(handler_record) };
    let handler_ipsr = unsafe { read_volatile(handler_record.add(1)) };
    let handler_nocp = unsafe { read_volatile(SCB_CFSR) } & CFSR_UFSR_NOCP;
    let handler_fp_ok = handler_fp_result == 0xC3
        && handler_ipsr == 11
        && handler_nocp == 0
        && thread_unprivileged == 1;
    send(b"cortex-m7 cpacr handler fp=");
    send(if handler_fp_ok { b"1" } else { b"0" });
    send(b" ipsr=");
    send_hex32(handler_ipsr);
    send(b" thread_unprivileged=");
    send_hex32(thread_unprivileged);
    send(b" nocp=");
    send_hex32(handler_nocp);
    send(b" result=");
    send_hex32(handler_fp_result);
    send(b"\r\n");
    ok && privileged_fp_ok
        && privileged_denied_ok
        && reserved_denied_ok
        && nocp_escalated_ok
        && priority_escalated_ok
        && handler_fp_ok
        && nocp_ok
        && full_access_ok
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

    // TXE remains set while the TDR is empty. Clearing NVIC pending without
    // changing TXEIE must let the USART level re-pend its interrupt.
    unsafe { write_volatile(NVIC_ICPR1, USART3_IRQ_BIT) };
    let mut txe_irq_reasserted = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT != 0 {
            txe_irq_reasserted = true;
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

    if txe_irq_isolated && txe_irq_asserted && txe_irq_reasserted && txe_irq_released {
        send(b"usart3 txe_irq=isolated,asserted,reasserted,released ok\r\n");
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

    // TC stays high until another TDR write. Clearing NVIC pending while
    // TCIE remains enabled must therefore let the USART level re-pend IRQ 39.
    unsafe { write_volatile(NVIC_ICPR1, USART3_IRQ_BIT) };
    let mut tc_irq_reasserted = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR1) } & USART3_IRQ_BIT != 0 {
            tc_irq_reasserted = true;
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

    if tc_irq_isolated && tc_irq_asserted && tc_irq_reasserted && tc_irq_released {
        send(b"usart3 tc_irq=isolated,asserted,reasserted,released ok\r\n");
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
    }

    #[cfg(feature = "nucleo-h753zi-rx")]
    unsafe {
        // Neither single enable can transmit these bytes. The profile's
        // receive-gate probe also starts with RE clear, so queued RX bytes
        // remain on the wire until both UE and RE are enabled below.
        write_volatile(USART_CR1, USART_UE);
        write_volatile(USART_TDR, b'!' as u32);
        write_volatile(USART_CR1, USART_TE);
        write_volatile(USART_TDR, b'?' as u32);
    }
    #[cfg(feature = "nucleo-h753zi-rx")]
    let rx_enable_gates_ok = check_uart_rx_enable_gates();
    #[cfg(not(feature = "nucleo-h753zi-rx"))]
    unsafe {
        write_volatile(USART_CR1, USART_UE | USART_TE | USART_RECEIVER_ENABLE);
    }

    set_led(false);
    send(b"STM32H7 register smoke\r\n");
    #[cfg(feature = "stm32h743-exception-frame")]
    {
        unsafe {
            write_volatile(SCB_CPACR, CPACR_PRIVILEGED_ONLY);
        }
        let cpacr_privileged = unsafe { read_volatile(SCB_CPACR) } & CPACR_CP10_CP11;
        unsafe {
            write_volatile(SCB_CPACR, CPACR_CP10_CP11);
        }
        let cpacr_full = unsafe { read_volatile(SCB_CPACR) } & CPACR_CP10_CP11;
        send(b"cortex-m7 cpacr priv=");
        send_hex32(cpacr_privileged);
        send(b" full=");
        send_hex32(cpacr_full);
        send(b"\r\n");
        let exception_ok = check_cortex_m7_exception_frame();
        send(if exception_ok {
            b"cortex-m7 exception ok\r\n"
        } else {
            b"cortex-m7 exception fail\r\n"
        });
    }
    #[cfg(feature = "nucleo-h753zi-rx")]
    send(if rx_enable_gates_ok {
        b"usart3 rx_gate re_off=1 ue_off=1 ok\r\n"
    } else {
        b"usart3 rx_gate fail fail\r\n"
    });
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
