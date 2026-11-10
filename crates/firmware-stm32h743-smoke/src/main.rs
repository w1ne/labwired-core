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
const NVIC_ISER0: *mut u32 = 0xE000_E100 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_ICER0: *mut u32 = 0xE000_E180 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_ISPR0: *const u32 = 0xE000_E200 as *const u32;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_ICPR0: *mut u32 = 0xE000_E280 as *mut u32;
#[cfg(feature = "stm32h743-exception-frame")]
const NVIC_IABR0: *const u32 = 0xE000_E300 as *const u32;
#[cfg(feature = "stm32h743-exception-frame")]
const EXCEPTION_IRQ: u32 = 5;
#[cfg(feature = "stm32h743-exception-frame")]
const EXCEPTION_IRQ_BIT: u32 = 1 << EXCEPTION_IRQ;
#[cfg(feature = "stm32h743-exception-frame")]
const EXCEPTION_VECTOR_INDEX: usize = 16 + EXCEPTION_IRQ as usize;

#[cfg(feature = "stm32h743-exception-frame")]
#[repr(C, align(1024))]
struct RamVectorTable([u32; 64]);

#[cfg(feature = "stm32h743-exception-frame")]
static mut RAM_VECTOR_TABLE: RamVectorTable = RamVectorTable([0; 64]);

// Handler scratch and the stacked frame captured before exception return.
#[cfg(feature = "stm32h743-exception-frame")]
static mut EXCEPTION_RECORD: [u32; 14] = [0; 14];

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

/// Capture the hardware-created Cortex-M7 frame before a compiler prologue
/// changes MSP. r4 is saved because it is not part of the basic exception
/// frame; caller-saved registers are restored by exception return.
#[cfg(feature = "stm32h743-exception-frame")]
#[unsafe(naked)]
unsafe extern "C" fn smoke_exception_handler() {
    core::arch::naked_asm!(
        "mrs r0, msp",
        "push {{r4}}",
        "ldr r1, ={record}",
        "str r0, [r1, #36]",
        "mov r2, lr",
        "str r2, [r1, #32]",
        "mov r4, r0",
        "movs r2, #8",
        "2:",
        "ldr r3, [r4]",
        "str r3, [r1]",
        "adds r4, #4",
        "adds r1, #4",
        "subs r2, #1",
        "bne 2b",
        "mrs r0, ipsr",
        "ldr r1, ={record}",
        "str r0, [r1, #40]",
        "ldr r2, =0xE000E300",
        "ldr r0, [r2]",
        "str r0, [r1, #44]",
        "ldr r2, =0xE000E200",
        "ldr r0, [r2]",
        "str r0, [r1, #48]",
        "pop {{r4}}",
        "bx lr",
        record = sym EXCEPTION_RECORD,
    );
}

/// Pend IRQ 5 after setting sentinels in every caller-saved register which
/// Cortex-M stacks automatically. The recorded pre-exception MSP is eight
/// bytes below the Rust caller's MSP because r4/r7 are saved here.
#[cfg(feature = "stm32h743-exception-frame")]
#[unsafe(naked)]
unsafe extern "C" fn pend_exception_irq_with_sentinels() {
    core::arch::naked_asm!(
        "push {{r4, r7}}",
        "mrs r7, msp",
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

#[cfg(feature = "stm32h743-exception-frame")]
fn check_cortex_m7_exception_frame() -> bool {
    let vector_table = unsafe { core::ptr::addr_of_mut!(RAM_VECTOR_TABLE.0).cast::<u32>() };
    let record = unsafe { core::ptr::addr_of_mut!(EXCEPTION_RECORD).cast::<u32>() };
    let source_vtor = unsafe { read_volatile(SCB_VTOR) };

    unsafe {
        write_volatile(NVIC_ICER0, EXCEPTION_IRQ_BIT);
        write_volatile(NVIC_ICPR0, EXCEPTION_IRQ_BIT);
        for index in 0..64 {
            write_volatile(vector_table.add(index), 0);
        }
        for index in 0..14 {
            write_volatile(record.add(index), 0);
        }
        let handler = (smoke_exception_handler as *const () as usize as u32) | 1;
        write_volatile(vector_table.add(EXCEPTION_VECTOR_INDEX), handler);
        write_volatile(SCB_VTOR, vector_table as u32);
        write_volatile(NVIC_ISER0, EXCEPTION_IRQ_BIT);
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

    unsafe {
        write_volatile(NVIC_ICER0, EXCEPTION_IRQ_BIT);
        write_volatile(NVIC_ICPR0, EXCEPTION_IRQ_BIT);
        write_volatile(SCB_VTOR, source_vtor);
    }
    let returned = unsafe { read_volatile(NVIC_ISPR0) } & EXCEPTION_IRQ_BIT == 0
        && unsafe { read_volatile(NVIC_IABR0) } & EXCEPTION_IRQ_BIT == 0;
    let vtor_restored = unsafe { read_volatile(SCB_VTOR) } == source_vtor;
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
    let ok = vector_installed
        && vtor_installed
        && frame_ok
        && active_and_pending_ok
        && returned
        && vtor_restored;

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
    send(b"\r\n");
    ok
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
