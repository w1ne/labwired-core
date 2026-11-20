// NPCX9 Renode-onboarding smoke. Keeps the UART1 banner, checks SMBus0..7,
// then exercises the internal timers: ITIM32_1..6 at 0x400B0000 + n * 0x2000 (NVIC 28, 27, 17,
// 43, 44, 45) and ITIM64 at 0x400BE000 (NVIC 58), as in Renode's
// nuvoton_npcx9.repl. Register layout from Zephyr reg_def.h (itim32_reg /
// itim64_reg): ITPRE byte +0x01, ITCTS byte +0x04, ITCNT +0x08, ITCNT64H +0x0C.
//
// 1. ITPRE and ITCNT hold their values while ITEN is clear.
// 2. 15 MHz APB2 / (ITPRE + 1) = 1 MHz: ITEN starts the down-count; clearing
//    ITEN stops it, restores the load value and clears TO_STS.
// 3. ITCNT = 99 with TO_IE: three timeouts reach the core on NVIC 28; the
//    handler clears TO_STS (write 1), as Zephyr's npcx_itim_evt_isr does.
// 4. ITIM32_2 with NVIC 27 disabled: TO_STS without TO_IE leaves the line
//    idle, setting TO_IE pends it, clearing TO_STS lets ICPR release it.
// 5. ITIM32_2..6 and ITIM64 each take one timeout IRQ on their own line.
// 6. ITIM64 holds a 64-bit load and borrows across the word boundary.
// 7. CKSEL selects the 32768 Hz LFCLK: the count moves ~460x slower.
//
// 0. SMBus0..7 at their Renode bases: reset state, enable, empty-bus NACK,
//    NEGACK W1C clear and STOP return to idle. The NPCX9 platform has no
//    attached off-chip I2C devices; the model's unit tests cover slave traffic.
//
// Then the GPIO ports: GPIO0..GPIOF at 0x40081000 + n * 0x2000 (Renode
// GPIOPort.NPCX_GPIO), byte registers from Zephyr reg_def.h gpio_reg.
// 8. Each of the 16 ports holds PDIR / PTYPE / PPULL / PPUD / PENVDD /
//    PLOCK_CTL values.
// 9. PDOUT written before PDIR (the gpio_npcx.c order) holds while the pin is
//    an input and reaches PDIN once PDIR makes it an output.
// 10. Pull-up / pull-down inputs read 1 / 0 on PDIN.
// 11. An open-drain output with pull-up reads 1 when PDOUT releases the pad
//     and 0 when PDOUT drives it low.
// 12. A set PLOCK_CTL bit keeps its PDOUT bit.
//
// Then NPCX MDMA1 at 0x40011100 copies UART1 URBUF into code RAM when UMDSL.ERD
// is set. The smoke script injects "m" into UART1, then the existing UART2..4
// checks use "bB", "cC", "dD" from cr_uart2..4.
//
// Then UART2..4 at 0x400E2000 / 0x400E4000 / 0x400E6000 (Renode
// UART.NPCX_UART). The smoke script injects "bB", "cC", "dD" into cr_uart2..4.
// 14. Each port reads TBE set; RBF is set only on its own port and URBUF
//     returns its own first byte. UART1 is empty after the MDMA check.
// 15. Each port transmits its own "cr_uartN tx" line through its UTBUF.
// 16. FIFO-mode interrupts, UART1..4 on NVIC 33 / 32 / 38 / 39: UFTSTS reads
//     NXMIP | TFIFO_EMPTY | TEMPTY_LVL 16; TEMPTY_EN pends the line and
//     clearing it releases ICPR. UART2..4's RNEMPTY_EN handler reads the
//     second byte, drops the IRQ level and clears RBF.
//
// Then MTC at 0x400B7000 on NVIC 5. 17. TTC advances once a second; with WIE
// and PT = TTC + 1, three IRQs arrive one TTC increment apart and the handler
// clears PTO and moves PT on. ITIM64 times both gaps at 1000 ms.
//
// Then TWD at 0x400D8000 on NVIC 31. 18. The smoke checks reset values,
// one-second T0 interrupts, lock/unlock, watchdog touch/countdown/stop, then
// watchdog reset on the tenth period. The second boot checks reset status.
//
// HFCG/LFCG at 0x400B5000/0x400B5100: 19. check register writes, status bits
// and locks before the watchdog expires.
//
// Plain memory windows: 20. code/data RAM aliases, image type, PM1..4 and
// battery-backed RAM signature (checked again on the second boot).
//
// SPIP at 0x400D2000: 21. check 8/16-bit transfers, status, EIR/EIW and NVIC 57.
// No SPI device is attached, so MISO reads 0.
// FIU at 0x40020000: 22. check UMA write-enable/status, page program, readback,
// NOR one-way programming and 64 KiB block erase. The direct XIP window is not mapped.
// Bare-register, no HAL.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// LabWired's NPCX UART model exposes UTBUF at +0x00 and the transmit-ready
// TBE flag at UICTRL +0x04. Clock and baud configuration are outside this
// smoke's modeled surface.
const UART1_BASE: usize = 0x400E_0000;
const UTBUF: *mut u8 = UART1_BASE as *mut u8;
const UICTRL: *const u8 = (UART1_BASE + 0x04) as *const u8;
const UMDSL: usize = 0x0A;
const UMDSL_ERD: u8 = 1 << 5;
const TBE: u8 = 1 << 0;
const RBF: u8 = 1 << 1;
const URBUF: usize = 0x02;
const UICTRL_OFF: usize = 0x04;
// (base, injected byte, name): UART2..4.
const UARTS: [(usize, u8, &[u8]); 3] = [
    (0x400E_2000, b'b', b"cr_uart2"),
    (0x400E_4000, b'c', b"cr_uart3"),
    (0x400E_6000, b'd', b"cr_uart4"),
];
// FIFO-mode registers (Zephyr reg_def.h uart_reg).
const UFTSTS: usize = 0x20;
const UFRSTS: usize = 0x22;
const UFTCTL: usize = 0x24;
const UFRCTL: usize = 0x26;
const UFTCTL_TEMPTY_EN: u8 = 1 << 6;
const UFRCTL_RNEMPTY_EN: u8 = 1 << 6;
// UART1..4 NVIC lines.
const UART_LINES: [(usize, u32); 4] = [
    (UART1_BASE, 33),
    (0x400E_2000, 32),
    (0x400E_4000, 38),
    (0x400E_6000, 39),
];
/// Byte the RX IRQ handler read from UART2..4 (index 0..2).
static UART_RX: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static UART_IRQS: AtomicU32 = AtomicU32::new(0);

const MTC: usize = 0x400B_7000;
const MTC_LINE: u32 = 5;
const TTC: usize = 0x00;
const WTC: usize = 0x04;
const WTC_WIE: u32 = 1 << 31;
const WTC_PTO: u32 = 1 << 30;
const PT_MASK: u32 = (1 << 25) - 1;
static MTC_IRQS: AtomicU32 = AtomicU32::new(0);
/// TTC and ITIM64 ITCNT (1 MHz down-count) at each MTC IRQ.
static MTC_TTC: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static MTC_US: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

const ITPRE: usize = 0x01;
const ITCTS: usize = 0x04;
const ITCNT: usize = 0x08;
const ITCNT64H: usize = 0x0C;
const TO_STS: u8 = 1 << 0;
const TO_IE: u8 = 1 << 2;
const CKSEL: u8 = 1 << 4;
const ITEN: u8 = 1 << 7;
// 15 MHz APB2 / 15 = 1 MHz.
const PRE_1MHZ: u8 = 14;

// (base, NVIC line): ITIM32_1..6, then ITIM64.
const TIMERS: [(usize, u32); 7] = [
    (0x400B_0000, 28),
    (0x400B_2000, 27),
    (0x400B_4000, 17),
    (0x400B_6000, 43),
    (0x400B_8000, 44),
    (0x400B_A000, 45),
    (0x400B_E000, 58),
];
const ITIM32_1: usize = TIMERS[0].0;
const ITIM32_2: usize = TIMERS[1].0;
const ITIM64: usize = TIMERS[6].0;

const GPIO0: usize = 0x4008_1000;
const GPIO_STRIDE: usize = 0x2000;
const PDOUT: usize = 0x00;
const PDIN: usize = 0x01;
const PDIR: usize = 0x02;
const PPULL: usize = 0x03;
const PPUD: usize = 0x04;
const PENVDD: usize = 0x05;
const PTYPE: usize = 0x06;
const PLOCK_CTL: usize = 0x07;

const NVIC_ISER: usize = 0xE000_E100;
const NVIC_ICER: usize = 0xE000_E180;
const NVIC_ISPR: usize = 0xE000_E200;
const NVIC_ICPR: usize = 0xE000_E280;

const TWD: usize = 0x400D_8000;
const TWD_LINE: u32 = 31;
const TWCFG: usize = 0x00;
const TWCP: usize = 0x02;
const TWDT0: usize = 0x04;
const T0CSR: usize = 0x06;
const WDCNT: usize = 0x08;
const WDSDM: usize = 0x0A;
const TWMWD: usize = 0x0E;
const TWCFG_LTWDT0: u8 = 1 << 2;
const TWCFG_WDSDME: u8 = 1 << 5;
const T0CSR_TC: u8 = 1 << 1;
const T0CSR_WDRST_STS: u8 = 1 << 4;
const T0CSR_WD_RUN: u8 = 1 << 5;
static TWD_IRQS: AtomicU32 = AtomicU32::new(0);
/// ITIM64 ITCNT at the first three TWD IRQs.
static TWD_US: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
/// Nonzero: the TWD handler prints a '.' per IRQ.
static TWD_DOTS: AtomicU32 = AtomicU32::new(0);

const HFCG: usize = 0x400B_5000;
const LFCG: usize = 0x400B_5100;
const HFCGCTRL: usize = 0x00;
const HFCGML: usize = 0x02;
const HFCGMH: usize = 0x04;
const HFCGN: usize = 0x06;
const HFCGP: usize = 0x08;
const HFCBCD: usize = 0x10;
const HFCBCD1: usize = 0x12;
const HFCBCD2: usize = 0x14;
const HFCBCD3: usize = 0x1D;
const CODE_RAM: usize = 0x1005_8000;
const CODE_RAM_ALIAS: usize = 0x2005_8000;
const CODE_RAM_LAST: usize = 0x6_7FFC;
const DATA_RAM_ALIAS: usize = 0x100C_0000;
const DATA_RAM: usize = 0x200C_0000;
const BBRAM: usize = 0x400A_F000;
const BBRAM_LAST: usize = 0x7C;
const BBRAM_SIG: u32 = 0xB0BA_CAFE;
const IMAGE_TYPE: usize = 0x4000_C009;
const PM_BASES: [usize; 4] = [0x400C_9000, 0x400C_B000, 0x400C_D000, 0x400C_F000];

const HFCGCTRL_LOAD: u8 = 1 << 0;
const HFCGCTRL_LOCK: u8 = 1 << 2;
const HFCGCTRL_CLK_CHNG: u8 = 1 << 7;
const LFCGCTL: usize = 0x00;
const HFRDI: usize = 0x02;
const LFCGCTL2: usize = 0x14;
const LFCGCTL_LFLOC: u8 = 1 << 5;
const LFCGCTL2_XT_OSC_SL_EN: u8 = 1 << 6;

const SPIP: usize = 0x400D_2000;
const SPIP_LINE: u32 = 57;
const SPIP_DATA: usize = 0x00;
const SPIP_CTL1: usize = 0x02;
const SPIP_STAT: usize = 0x04;
const CTL1_SPIEN: u16 = 1 << 0;
const CTL1_MOD: u16 = 1 << 2;
const CTL1_EIR: u16 = 1 << 5;
const CTL1_EIW: u16 = 1 << 6;
const STAT_BSY: u8 = 1 << 0;
const STAT_RBF: u8 = 1 << 1;
static SPIP_RD_IRQS: AtomicU32 = AtomicU32::new(0);
static SPIP_WR_IRQS: AtomicU32 = AtomicU32::new(0);

const FIU: usize = 0x4002_0000;
const FIU_CODE: usize = 0x16;
const FIU_ADDR0: usize = 0x17;
const FIU_DATA0: usize = 0x1A;
const FIU_CTS: usize = 0x1E;
const FIU_ECTS: usize = 0x1F;
const FIU_A_SIZE: u8 = 1 << 3;
const FIU_WRITE: u8 = 1 << 5;
const FIU_START: u8 = 1 << 7;

const MDMA1: usize = 0x4001_1100;
const MDMA_CTRL: usize = 0x00;
const MDMA_SOURCE: usize = 0x04;
const MDMA_DESTINATION: usize = 0x08;
const MDMA_TRANSFER_COUNT: usize = 0x0C;
const MDMA_CURRENT_DESTINATION: usize = 0x14;
const MDMA_CURRENT_COUNT: usize = 0x18;
const MDMAEN: u32 = 1;
const MDMA_TC: u32 = 1 << 18;
const MDMA_RX_DESTINATION: usize = 0x1005_F000;

const SMBUS_BASES: [usize; 8] = [
    0x4000_9000,
    0x4000_B000,
    0x400C_0000,
    0x400C_2000,
    0x4000_8000,
    0x4001_7000,
    0x4001_8000,
    0x4001_9000,
];
const SMBUS_STATUS: usize = 0x02;
const SMBUS_CONTROL_STATUS: usize = 0x04;
const SMBUS_CONTROL1: usize = 0x06;
const SMBUS_CONTROL2: usize = 0x0A;
const SMBUS_START: u8 = 1 << 0;
const SMBUS_STOP: u8 = 1 << 1;
const SMBUS_INTEN: u8 = 1 << 2;
const SMBUS_NEGACK: u8 = 1 << 4;

static ITIM1_IRQS: AtomicU32 = AtomicU32::new(0);
/// Bit n set: TIMERS[n] took its IRQ.
static SEEN: AtomicU32 = AtomicU32::new(0);

fn rd8(base: usize, offset: usize) -> u8 {
    unsafe { read_volatile((base + offset) as *const u8) }
}

fn wr8(base: usize, offset: usize, value: u8) {
    unsafe { write_volatile((base + offset) as *mut u8, value) }
}

fn rd16(base: usize, offset: usize) -> u16 {
    unsafe { read_volatile((base + offset) as *const u16) }
}

fn wr16(base: usize, offset: usize, value: u16) {
    unsafe { write_volatile((base + offset) as *mut u16, value) }
}

fn rd32(base: usize, offset: usize) -> u32 {
    unsafe { read_volatile((base + offset) as *const u32) }
}

fn wr32(base: usize, offset: usize, value: u32) {
    unsafe { write_volatile((base + offset) as *mut u32, value) }
}

fn fiu_exec(code: u8, address: Option<u32>, data: u8, control: u8) -> u8 {
    wr8(FIU, FIU_CODE, code);
    if let Some(address) = address {
        for index in 0..3 {
            wr8(FIU, FIU_ADDR0 + index, (address >> (index * 8)) as u8);
        }
    }
    wr8(FIU, FIU_DATA0, data);
    wr8(FIU, FIU_CTS, control | FIU_START);
    rd8(FIU, FIU_DATA0)
}

fn nvic(reg: usize, line: u32) -> (usize, u32) {
    (reg + 4 * (line as usize / 32), 1 << (line % 32))
}

fn nvic_set(reg: usize, line: u32) {
    let (addr, bit) = nvic(reg, line);
    unsafe { write_volatile(addr as *mut u32, bit) }
}

fn nvic_pending(line: u32) -> bool {
    let (addr, bit) = nvic(NVIC_ISPR, line);
    unsafe { read_volatile(addr as *const u32) & bit != 0 }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UICTRL) & TBE == 0 {}
        write_volatile(UTBUF, byte);
    }
}

fn puts(s: &[u8]) {
    for byte in s {
        putc(*byte);
    }
}

fn put_kv(key: &[u8], value: u32) {
    putc(b' ');
    puts(key);
    putc(b'=');
    let mut buf = [0u8; 10];
    let mut n = value;
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    puts(&buf[i..]);
}

fn uart_puts(base: usize, s: &[u8]) {
    for byte in s {
        while rd8(base, UICTRL_OFF) & TBE == 0 {}
        wr8(base, 0x00, *byte);
    }
}

fn spin(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}

fn check_smbus_controllers() {
    let mut reset = 0u32;
    let mut enabled = 0u32;
    let mut nacked = 0u32;
    let mut cleared = 0u32;
    let mut stopped = 0u32;
    for &base in &SMBUS_BASES {
        reset += (rd8(base, SMBUS_STATUS) == 0
            && rd8(base, SMBUS_CONTROL_STATUS) == 0xC0
            && rd8(base, SMBUS_CONTROL2) == 0) as u32;

        wr8(base, SMBUS_CONTROL2, 1); // module enable
        enabled += (rd8(base, SMBUS_CONTROL2) == 1) as u32;
        wr8(base, SMBUS_CONTROL1, SMBUS_INTEN | SMBUS_START);
        wr8(base, 0, 0xA0); // address 0x50 on the intentionally empty SMBus
        nacked += (rd8(base, SMBUS_STATUS) == (SMBUS_NEGACK | (1 << 6))) as u32;
        wr8(base, SMBUS_STATUS, SMBUS_NEGACK); // W1C
        cleared += (rd8(base, SMBUS_STATUS) == (1 << 6)) as u32;
        wr8(base, SMBUS_CONTROL1, SMBUS_INTEN | SMBUS_STOP);
        stopped += (rd8(base, SMBUS_STATUS) == 0 && rd8(base, SMBUS_CONTROL_STATUS) == 0xC0) as u32;
    }
    puts(b"smbus");
    put_kv(b"reset", reset);
    put_kv(b"en", enabled);
    put_kv(b"nack", nacked);
    put_kv(b"clear", cleared);
    put_kv(b"stop", stopped);
    puts(
        if reset == 8 && enabled == 8 && nacked == 8 && cleared == 8 && stopped == 8 {
            b" ok\r\n"
        } else {
            b" FAIL\r\n"
        },
    );
}

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    for (n, &(base, line)) in TIMERS.iter().enumerate() {
        if irqn as u32 == line {
            wr8(base, ITCTS, rd8(base, ITCTS) | TO_STS);
            if n == 0 {
                ITIM1_IRQS.fetch_add(1, Ordering::SeqCst);
            }
            SEEN.fetch_or(1 << n, Ordering::SeqCst);
        }
    }
    // MTC: record TTC and the ITIM64 time, clear PTO, move PT on.
    if irqn as u32 == MTC_LINE {
        let n = MTC_IRQS.fetch_add(1, Ordering::SeqCst) as usize;
        let ttc = rd32(MTC, TTC);
        if n < 3 {
            MTC_TTC[n].store(ttc, Ordering::SeqCst);
            MTC_US[n].store(rd32(ITIM64, ITCNT), Ordering::SeqCst);
        }
        wr32(
            MTC,
            WTC,
            WTC_WIE | WTC_PTO | (ttc.wrapping_add(1) & PT_MASK),
        );
    }
    // TWD T0 terminal count.
    if irqn as u32 == TWD_LINE {
        let n = TWD_IRQS.fetch_add(1, Ordering::SeqCst) as usize;
        if n < 3 {
            TWD_US[n].store(rd32(ITIM64, ITCNT), Ordering::SeqCst);
        }
        if TWD_DOTS.load(Ordering::SeqCst) != 0 {
            putc(b'.');
        }
    }
    // SPIP, as the Zephyr ISR: RBF clear -> write ready, drop EIW; RBF set
    // -> read DATA, drop EIR.
    if irqn as u32 == SPIP_LINE {
        if rd8(SPIP, SPIP_STAT) & STAT_RBF == 0 {
            wr16(SPIP, SPIP_CTL1, rd16(SPIP, SPIP_CTL1) & !CTL1_EIW);
            SPIP_WR_IRQS.fetch_add(1, Ordering::SeqCst);
        } else {
            let _ = rd16(SPIP, SPIP_DATA);
            wr16(SPIP, SPIP_CTL1, rd16(SPIP, SPIP_CTL1) & !CTL1_EIR);
            SPIP_RD_IRQS.fetch_add(1, Ordering::SeqCst);
        }
    }
    // UART2..4 RX: read URBUF, as uart_npcx_fifo_read does.
    for (n, &(base, line)) in UART_LINES[1..].iter().enumerate() {
        if irqn as u32 == line {
            UART_RX[n].store(u32::from(rd8(base, URBUF)), Ordering::SeqCst);
            UART_IRQS.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[entry]
fn main() -> ! {
    // Second boot, after the watchdog reset at the end of check 17.
    let csr = rd8(TWD, T0CSR);
    if csr & T0CSR_WDRST_STS != 0 {
        let stopped = (csr & T0CSR_WD_RUN == 0) as u32;
        wr8(TWD, T0CSR, T0CSR_WDRST_STS);
        let clr = (rd8(TWD, T0CSR) & T0CSR_WDRST_STS == 0) as u32;
        let t0 = (rd16(TWD, TWDT0) == 0xFFFF) as u32;
        puts(b"|twd reboot wdrst=1");
        put_kv(b"stopped", stopped);
        put_kv(b"clr", clr);
        put_kv(b"t0", t0);
        puts(if stopped == 1 && clr == 1 && t0 == 1 {
            b" ok\r\n"
        } else {
            b" FAIL\r\n"
        });
        // 20, second boot: the BBRAM signature survived the reset.
        let kept = (rd32(BBRAM, 0) == BBRAM_SIG && rd32(BBRAM, BBRAM_LAST) == !BBRAM_SIG) as u32;
        puts(b"bbram");
        put_kv(b"kept", kept);
        puts(if kept == 1 { b" ok\r\n" } else { b" FAIL\r\n" });
        loop {
            unsafe { core::arch::asm!("wfi") };
        }
    }

    puts(b"NPCX9 UART1 smoke\r\n");

    // The imported Renode platform has eight empty SMBus controllers. Verify
    // each reset register bank, enable, address NACK, W1C clear and STOP.
    check_smbus_controllers();

    // 1. Registers hold while disabled.
    let t1 = ITIM32_1;
    wr8(t1, ITPRE, PRE_1MHZ);
    wr32(t1, ITCNT, 0x1234_5678);
    let regs = (rd8(t1, ITPRE) == PRE_1MHZ && rd32(t1, ITCNT) == 0x1234_5678) as u32;

    // 2. Down-count, stop.
    wr32(t1, ITCNT, 0xFFFF_FFFF);
    wr8(t1, ITCTS, ITEN);
    let en = (rd8(t1, ITCTS) & ITEN != 0) as u32;
    spin(2000);
    let run = (rd32(t1, ITCNT) < 0xFFFF_FFFF) as u32;
    wr8(t1, ITCTS, 0);
    let stop = (rd32(t1, ITCNT) == 0xFFFF_FFFF && rd8(t1, ITCTS) == 0) as u32;

    // 3. Three timeout IRQs on NVIC 28.
    wr32(t1, ITCNT, 99);
    nvic_set(NVIC_ISER, TIMERS[0].1);
    wr8(t1, ITCTS, ITEN | TO_IE);
    let mut guard = 0u32;
    while ITIM1_IRQS.load(Ordering::SeqCst) < 3 && guard < 200_000 {
        guard += 1;
    }
    wr8(t1, ITCTS, 0);
    nvic_set(NVIC_ICER, TIMERS[0].1);
    let irq = ITIM1_IRQS.load(Ordering::SeqCst);

    // 4. Level on ITIM32_2 with NVIC 27 disabled.
    let t2 = ITIM32_2;
    let l2 = TIMERS[1].1;
    nvic_set(NVIC_ICER, l2);
    nvic_set(NVIC_ICPR, l2);
    wr8(t2, ITPRE, PRE_1MHZ);
    wr32(t2, ITCNT, 9);
    wr8(t2, ITCTS, ITEN);
    let mut guard = 0u32;
    while rd8(t2, ITCTS) & TO_STS == 0 && guard < 100_000 {
        guard += 1;
    }
    wr8(t2, ITCTS, 0);
    // Disabling clears TO_STS; re-run once to leave it set while stopped.
    let sts_cleared = (rd8(t2, ITCTS) & TO_STS == 0) as u32;
    wr8(t2, ITCTS, ITEN);
    let mut guard = 0u32;
    while rd8(t2, ITCTS) & TO_STS == 0 && guard < 100_000 {
        guard += 1;
    }
    let sts = (rd8(t2, ITCTS) & TO_STS != 0) as u32;
    let masked = (!nvic_pending(l2)) as u32;
    wr8(t2, ITCTS, ITEN | TO_IE);
    let pend = nvic_pending(l2) as u32;
    wr8(t2, ITCTS, TO_IE | TO_STS);
    nvic_set(NVIC_ICPR, l2);
    let rel = (!nvic_pending(l2)) as u32;
    wr8(t2, ITCTS, 0);

    // 5. One IRQ on each remaining line.
    SEEN.store(0, Ordering::SeqCst);
    for (n, &(base, line)) in TIMERS.iter().enumerate().skip(1) {
        wr8(base, ITPRE, PRE_1MHZ);
        wr32(base, ITCNT, 49);
        if n == 6 {
            wr32(base, ITCNT64H, 0);
        }
        nvic_set(NVIC_ICPR, line);
        nvic_set(NVIC_ISER, line);
        wr8(base, ITCTS, ITEN | TO_IE);
        let mut guard = 0u32;
        while SEEN.load(Ordering::SeqCst) & (1 << n) == 0 && guard < 100_000 {
            guard += 1;
        }
        wr8(base, ITCTS, 0);
        nvic_set(NVIC_ICER, line);
    }
    let each = SEEN.load(Ordering::SeqCst).count_ones();

    // 6. ITIM64: 0x1_0000_0005 borrows into the high word after six ticks.
    let t64 = ITIM64;
    wr32(t64, ITCNT, 5);
    wr32(t64, ITCNT64H, 1);
    let w64 = (rd32(t64, ITCNT64H) == 1 && rd32(t64, ITCNT) == 5) as u32;
    wr8(t64, ITCTS, ITEN);
    spin(200);
    let hi = rd32(t64, ITCNT64H);
    let lo = rd32(t64, ITCNT);
    wr8(t64, ITCTS, 0);
    let borrow = (hi == 0 && lo > 0xFF00_0000) as u32;

    // 7. LFCLK: 15 MHz / 32768 Hz is ~458 cycles per tick.
    wr8(t1, ITPRE, 0);
    wr32(t1, ITCNT, 1000);
    wr8(t1, ITCTS, ITEN | CKSEL);
    spin(2000);
    let gone = 1000 - rd32(t1, ITCNT);
    wr8(t1, ITCTS, 0);
    let lfclk = (gone >= 1 && gone <= 50) as u32;

    puts(b"itim");
    put_kv(b"regs", regs);
    put_kv(b"en", en);
    put_kv(b"run", run);
    put_kv(b"stop", stop);
    put_kv(b"irq", irq);
    put_kv(b"sts_cleared", sts_cleared);
    put_kv(b"sts", sts);
    put_kv(b"masked", masked);
    put_kv(b"pend", pend);
    put_kv(b"rel", rel);
    put_kv(b"each", each);
    put_kv(b"w64", w64);
    put_kv(b"borrow", borrow);
    put_kv(b"lfclk", lfclk);
    let ok = regs == 1
        && en == 1
        && run == 1
        && stop == 1
        && irq == 3
        && sts_cleared == 1
        && sts == 1
        && masked == 1
        && pend == 1
        && rel == 1
        && each == 6
        && w64 == 1
        && borrow == 1
        && lfclk == 1;
    puts(if ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 8. All 16 ports exist and hold their configuration.
    let mut ports = 0u32;
    for n in 0..16 {
        let base = GPIO0 + n * GPIO_STRIDE;
        let tag = 0xA0 | n as u8;
        let cfg = [PDIR, PPULL, PPUD, PENVDD, PTYPE, PLOCK_CTL];
        for (k, &reg) in cfg.iter().enumerate() {
            wr8(base, reg, tag ^ k as u8);
        }
        if cfg
            .iter()
            .enumerate()
            .all(|(k, &reg)| rd8(base, reg) == tag ^ k as u8)
        {
            ports += 1;
        }
        for &reg in cfg.iter().rev() {
            wr8(base, reg, 0);
        }
    }

    // 9-12 on GPIO5.
    let g = GPIO0 + 5 * GPIO_STRIDE;
    wr8(g, PDOUT, 0x05);
    let latch = (rd8(g, PDOUT) == 0x05 && rd8(g, PDIN) & 0x0F == 0) as u32;
    wr8(g, PDIR, 0x0F);
    let drive = (rd8(g, PDIN) & 0x0F == 0x05) as u32;
    wr8(g, PDIR, 0);
    wr8(g, PDOUT, 0);
    wr8(g, PPULL, 0x30);
    wr8(g, PPUD, 0x20);
    let pull = (rd8(g, PDIN) & 0x30 == 0x10) as u32;
    wr8(g, PPULL, 0x40);
    wr8(g, PPUD, 0);
    wr8(g, PTYPE, 0x40);
    wr8(g, PDOUT, 0x40);
    wr8(g, PDIR, 0x40);
    let od_high = rd8(g, PDIN) & 0x40 != 0;
    wr8(g, PDOUT, 0);
    let od_low = rd8(g, PDIN) & 0x40 == 0;
    let od = (od_high && od_low) as u32;
    wr8(g, PDOUT, 0x80);
    wr8(g, PLOCK_CTL, 0x80);
    wr8(g, PDOUT, 0x01);
    let lock = (rd8(g, PDOUT) == 0x81) as u32;
    puts(b"gpio");
    put_kv(b"ports", ports);
    put_kv(b"latch", latch);
    put_kv(b"drive", drive);
    put_kv(b"pull", pull);
    put_kv(b"od", od);
    put_kv(b"lock", lock);
    let gpio_ok = ports == 16 && latch == 1 && drive == 1 && pull == 1 && od == 1 && lock == 1;
    puts(if gpio_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 13. NPCX MDMA1 UART1 RX: the smoke script injected one byte into UART1.
    // UMDSL.ERD requests context 0, which copies fixed URBUF to memory.
    let mdma_source = (rd32(MDMA1, MDMA_SOURCE) == (UART1_BASE + URBUF) as u32) as u32;
    wr32(MDMA1, MDMA_DESTINATION, MDMA_RX_DESTINATION as u32);
    wr32(MDMA1, MDMA_TRANSFER_COUNT, 1);
    wr8(UART1_BASE, UMDSL, UMDSL_ERD);
    wr32(MDMA1, MDMA_CTRL, MDMAEN);
    let mut mdma_guard = 0u32;
    while rd32(MDMA1, MDMA_CTRL) & MDMA_TC == 0 && mdma_guard < 100_000 {
        mdma_guard += 1;
        core::hint::spin_loop();
    }
    let mdma_data = (rd8(MDMA_RX_DESTINATION, 0) == b'm') as u32;
    let mdma_dest = (rd32(MDMA1, MDMA_CURRENT_DESTINATION) == MDMA_RX_DESTINATION as u32) as u32;
    let mdma_count =
        (rd32(MDMA1, MDMA_TRANSFER_COUNT) == 0 && rd32(MDMA1, MDMA_CURRENT_COUNT) == 0) as u32;
    let mdma_tc = (rd32(MDMA1, MDMA_CTRL) & MDMA_TC != 0) as u32;
    let mdma_fifo = (rd8(UART1_BASE, UICTRL_OFF) & RBF == 0) as u32;
    wr8(UART1_BASE, UMDSL, 0);
    puts(b"mdma");
    put_kv(b"src", mdma_source);
    put_kv(b"data", mdma_data);
    put_kv(b"dst", mdma_dest);
    put_kv(b"count", mdma_count);
    put_kv(b"tc", mdma_tc);
    put_kv(b"fifo", mdma_fifo);
    let mdma_ok = mdma_source == 1
        && mdma_data == 1
        && mdma_dest == 1
        && mdma_count == 1
        && mdma_tc == 1
        && mdma_fifo == 1;
    puts(if mdma_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 14. Per-port TBE / RBF / URBUF.
    let mut tbe = 0;
    let mut rx = 0;
    for &(base, byte, _) in UARTS.iter() {
        let ctrl = rd8(base, UICTRL_OFF);
        tbe += (ctrl & TBE != 0) as u32;
        if ctrl & RBF != 0 && rd8(base, URBUF) == byte {
            rx += 1;
        }
    }
    let isolated = (unsafe { read_volatile(UICTRL) } & RBF == 0) as u32;
    // 15. Per-port transmit.
    for &(base, _, name) in UARTS.iter() {
        uart_puts(base, name);
        uart_puts(base, b" tx\r\n");
    }
    puts(b"uarts");
    put_kv(b"tbe", tbe);
    put_kv(b"rx", rx);
    put_kv(b"isolated", isolated);
    let uart_ok = tbe == 3 && rx == 3 && isolated == 1;
    puts(if uart_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 16. FIFO-mode TX / RX interrupts.
    let mut fts = 0;
    let mut txp = 0;
    let mut txrel = 0;
    for &(base, line) in UART_LINES.iter() {
        fts += (rd8(base, UFTSTS) == 0xD0) as u32;
        nvic_set(NVIC_ICER, line);
        nvic_set(NVIC_ICPR, line);
        wr8(base, UFTCTL, rd8(base, UFTCTL) | UFTCTL_TEMPTY_EN);
        spin(20);
        txp += nvic_pending(line) as u32;
        wr8(base, UFTCTL, rd8(base, UFTCTL) & !UFTCTL_TEMPTY_EN);
        spin(20);
        nvic_set(NVIC_ICPR, line);
        spin(20);
        txrel += (!nvic_pending(line)) as u32;
    }
    let mut frs = 0;
    let mut rxirq = 0;
    let mut drained = 0;
    for (n, (&(base, line), &(_, byte, _))) in UART_LINES[1..].iter().zip(UARTS.iter()).enumerate()
    {
        // One byte left, RFULL_LVL_SEL = 1: NEMPTY | FULL_LVL | count 1.
        frs += (rd8(base, UFRSTS) == 0x61) as u32;
        nvic_set(NVIC_ICPR, line);
        nvic_set(NVIC_ISER, line);
        wr8(base, UFRCTL, rd8(base, UFRCTL) | UFRCTL_RNEMPTY_EN);
        let mut guard = 0;
        while UART_RX[n].load(Ordering::SeqCst) == 0 && guard < 10_000 {
            guard += 1;
        }
        spin(50);
        nvic_set(NVIC_ICER, line);
        wr8(base, UFRCTL, rd8(base, UFRCTL) & !UFRCTL_RNEMPTY_EN);
        rxirq += (UART_RX[n].load(Ordering::SeqCst) == u32::from(byte.to_ascii_uppercase())) as u32;
        drained += (rd8(base, UICTRL_OFF) & RBF == 0 && rd8(base, UFRSTS) & 0x40 == 0) as u32;
    }
    let hits = UART_IRQS.load(Ordering::SeqCst);
    puts(b"uart irq");
    put_kv(b"fts", fts);
    put_kv(b"txp", txp);
    put_kv(b"txrel", txrel);
    put_kv(b"frs", frs);
    put_kv(b"rx", rxirq);
    put_kv(b"hits", hits);
    put_kv(b"drained", drained);
    let irq_ok =
        fts == 4 && txp == 4 && txrel == 4 && frs == 3 && rxirq == 3 && hits == 3 && drained == 3;
    puts(if irq_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 17. MTC.
    wr32(MTC, TTC, 0x1000_0000);
    let ttc_rw = (rd32(MTC, TTC).wrapping_sub(0x1000_0000) <= 1) as u32;
    wr32(MTC, WTC, 0xFFFF_FFFF & !(WTC_WIE | WTC_PTO));
    let pt_rw = (rd32(MTC, WTC) == PT_MASK) as u32;
    // ITIM64 free-running at 1 MHz, no IRQ.
    wr8(ITIM64, ITCTS, 0);
    wr8(ITIM64, ITPRE, PRE_1MHZ);
    wr32(ITIM64, ITCNT, 0xFFFF_FFFF);
    wr32(ITIM64, ITCNT64H, 0);
    wr8(ITIM64, ITCTS, ITEN);
    nvic_set(NVIC_ICPR, MTC_LINE);
    nvic_set(NVIC_ISER, MTC_LINE);
    let ttc0 = rd32(MTC, TTC);
    wr32(
        MTC,
        WTC,
        WTC_WIE | WTC_PTO | (ttc0.wrapping_add(1) & PT_MASK),
    );
    let mut guard = 0u32;
    while MTC_IRQS.load(Ordering::SeqCst) < 3 && guard < 10 {
        unsafe { core::arch::asm!("wfi") };
        guard += 1;
    }
    wr32(MTC, WTC, 0);
    nvic_set(NVIC_ICER, MTC_LINE);
    wr8(ITIM64, ITCTS, 0);
    let mtc_irqs = MTC_IRQS.load(Ordering::SeqCst);
    let t = |n: usize| MTC_TTC[n].load(Ordering::SeqCst);
    let us = |n: usize| MTC_US[n].load(Ordering::SeqCst);
    let steps = (t(0) == ttc0.wrapping_add(1) && t(1) == t(0) + 1 && t(2) == t(1) + 1) as u32;
    let ms1 = (us(0).wrapping_sub(us(1)) + 500) / 1000;
    let ms2 = (us(1).wrapping_sub(us(2)) + 500) / 1000;
    puts(b"mtc");
    put_kv(b"ttc", ttc_rw);
    put_kv(b"pt", pt_rw);
    put_kv(b"irq", mtc_irqs);
    put_kv(b"steps", steps);
    put_kv(b"ms1", ms1);
    put_kv(b"ms2", ms2);
    let mtc_ok =
        ttc_rw == 1 && pt_rw == 1 && mtc_irqs == 3 && steps == 1 && ms1 == 1000 && ms2 == 1000;
    puts(if mtc_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 18. TWD.
    let regs = (rd8(TWD, TWCFG) == 0
        && rd16(TWD, TWDT0) == 0xFFFF
        && rd8(TWD, WDCNT) == 0x0F
        && rd8(TWD, T0CSR) & (T0CSR_WD_RUN | T0CSR_WDRST_STS) == 0) as u32;
    // 1 s T0 period, timed with ITIM64 at 1 MHz.
    wr8(TWD, TWCP, 3);
    wr16(TWD, TWDT0, 4095);
    let t0rw = (rd8(TWD, TWCP) == 3 && rd16(TWD, TWDT0) == 4095) as u32;
    wr8(ITIM64, ITCTS, 0);
    wr8(ITIM64, ITPRE, PRE_1MHZ);
    wr32(ITIM64, ITCNT, 0xFFFF_FFFF);
    wr32(ITIM64, ITCNT64H, 0);
    wr8(ITIM64, ITCTS, ITEN);
    nvic_set(NVIC_ICPR, TWD_LINE);
    nvic_set(NVIC_ISER, TWD_LINE);
    let mut guard = 0u32;
    while TWD_IRQS.load(Ordering::SeqCst) < 3 && guard < 10 {
        unsafe { core::arch::asm!("wfi") };
        guard += 1;
    }
    nvic_set(NVIC_ICER, TWD_LINE);
    wr8(ITIM64, ITCTS, 0);
    let t0irq = TWD_IRQS.load(Ordering::SeqCst);
    let us = |n: usize| TWD_US[n].load(Ordering::SeqCst);
    let tms1 = (us(0).wrapping_sub(us(1)) + 500) / 1000;
    let tms2 = (us(1).wrapping_sub(us(2)) + 500) / 1000;
    let tc = (rd8(TWD, T0CSR) & T0CSR_TC != 0 && rd8(TWD, T0CSR) & T0CSR_TC == 0) as u32;
    // Lock, then unlock with the watchdog stopped.
    wr8(TWD, TWCFG, TWCFG_LTWDT0);
    wr16(TWD, TWDT0, 7);
    let lock = (rd16(TWD, TWDT0) == 0 && rd8(TWD, TWCFG) == TWCFG_LTWDT0) as u32;
    for b in [0x87, 0x61, 0x63] {
        wr8(TWD, WDSDM, b);
    }
    let unlock = (rd8(TWD, TWCFG) == 0 && rd16(TWD, TWDT0) == 4095) as u32;
    // 32-tick T0 period (~0.98 ms); the watchdog counts T0 periods.
    wr8(TWD, TWCP, 0);
    wr16(TWD, TWDT0, 31);
    wr8(TWD, TWCFG, TWCFG_WDSDME);
    wr8(TWD, WDCNT, 10);
    let run = (rd8(TWD, T0CSR) & T0CSR_WD_RUN != 0) as u32;
    let wait = |periods: u32| {
        let target = TWD_IRQS.load(Ordering::SeqCst) + periods;
        while TWD_IRQS.load(Ordering::SeqCst) < target {
            unsafe { core::arch::asm!("wfi") };
        }
    };
    nvic_set(NVIC_ICPR, TWD_LINE);
    nvic_set(NVIC_ISER, TWD_LINE);
    for _ in 0..6 {
        wait(5);
        wr8(TWD, WDSDM, 0x5C);
    }
    let touch = (rd8(TWD, T0CSR) & T0CSR_WD_RUN != 0) as u32;
    wait(3);
    let dec = (rd8(TWD, TWMWD) == 7) as u32;
    // Stop: the sequence with the watchdog running.
    for b in [0x87, 0x61, 0x63] {
        wr8(TWD, WDSDM, b);
    }
    let stopped = rd8(TWD, T0CSR) & T0CSR_WD_RUN == 0;
    wait(15);
    let stop = stopped as u32;
    puts(b"twd");
    put_kv(b"regs", regs);
    put_kv(b"t0", t0rw);
    put_kv(b"irq", t0irq);
    put_kv(b"ms1", tms1);
    put_kv(b"ms2", tms2);
    put_kv(b"tc", tc);
    put_kv(b"lock", lock);
    put_kv(b"unlock", unlock);
    put_kv(b"run", run);
    put_kv(b"touch", touch);
    put_kv(b"dec", dec);
    put_kv(b"stop", stop);
    let twd_ok = regs == 1
        && t0rw == 1
        && t0irq == 3
        && tms1 == 1000
        && tms2 == 1000
        && tc == 1
        && lock == 1
        && unlock == 1
        && run == 1
        && touch == 1
        && dec == 1
        && stop == 1;
    puts(if twd_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 19. HFCG / LFCG.
    wr8(HFCG, HFCGN, 0x02);
    wr8(HFCG, HFCGML, 0xBA);
    wr8(HFCG, HFCGMH, 0x0A);
    wr8(HFCG, HFCGCTRL, rd8(HFCG, HFCGCTRL) | HFCGCTRL_LOAD);
    let mut guard = 0u32;
    while rd8(HFCG, HFCGCTRL) & HFCGCTRL_CLK_CHNG != 0 && guard < 1000 {
        guard += 1;
    }
    let load = (guard < 1000
        && rd8(HFCG, HFCGCTRL) & HFCGCTRL_LOAD == 0
        && rd8(HFCG, HFCGN) == 0x02
        && rd8(HFCG, HFCGML) == 0xBA
        && rd8(HFCG, HFCGMH) == 0x0A) as u32;
    // HFCGP 0x5F: bits 2..3 reserved. HFCBCD 0xFF: only bits 1, 4..5.
    wr8(HFCG, HFCGP, 0x5F);
    wr8(HFCG, HFCBCD, 0xFF);
    wr8(HFCG, HFCBCD1, 0x11);
    wr8(HFCG, HFCBCD2, 0x22);
    wr8(HFCG, HFCBCD3, 0x01);
    let div = (rd8(HFCG, HFCGP) == 0x53
        && rd8(HFCG, HFCBCD) == 0x32
        && rd8(HFCG, HFCBCD1) == 0x11
        && rd8(HFCG, HFCBCD2) == 0x22
        && rd8(HFCG, HFCBCD3) == 0x01) as u32;
    wr8(HFCG, HFCGCTRL, HFCGCTRL_LOCK);
    wr8(HFCG, HFCGML, 0x55);
    let hlock = (rd8(HFCG, HFCGML) == 0xBA && rd8(HFCG, HFCGCTRL) == HFCGCTRL_LOCK) as u32;
    wr16(LFCG, HFRDI, 0xFFFF);
    wr8(LFCG, LFCGCTL2, LFCGCTL2_XT_OSC_SL_EN);
    let lf = (rd16(LFCG, HFRDI) == 0x0FFF && rd8(LFCG, LFCGCTL2) == LFCGCTL2_XT_OSC_SL_EN) as u32;
    wr8(LFCG, LFCGCTL, LFCGCTL_LFLOC);
    wr8(LFCG, LFCGCTL2, 0);
    let llock = (rd8(LFCG, LFCGCTL2) == LFCGCTL2_XT_OSC_SL_EN) as u32;
    puts(b"cdcg");
    put_kv(b"load", load);
    put_kv(b"div", div);
    put_kv(b"hlock", hlock);
    put_kv(b"lf", lf);
    put_kv(b"llock", llock);
    let cdcg_ok = load == 1 && div == 1 && hlock == 1 && lf == 1 && llock == 1;
    puts(if cdcg_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 20. Memory windows.
    wr32(CODE_RAM, 0, 0x1357_9BDF);
    wr32(CODE_RAM_ALIAS, CODE_RAM_LAST, 0x2468_ACE0);
    let code = (rd32(CODE_RAM_ALIAS, 0) == 0x1357_9BDF
        && rd32(CODE_RAM, CODE_RAM_LAST) == 0x2468_ACE0) as u32;
    // 64 KiB into data RAM: clear of .data/.bss below and the stack above.
    let probe = 0x1_0000;
    wr32(DATA_RAM_ALIAS, probe, 0x5A5A_A5A5);
    let data = (rd32(DATA_RAM, probe) == 0x5A5A_A5A5) as u32;
    wr8(IMAGE_TYPE, 0, 0x3C);
    let image = (rd8(IMAGE_TYPE, 0) == 0x3C) as u32;
    let mut pm = 0u32;
    for (i, &base) in PM_BASES.iter().enumerate() {
        wr8(base, 0, 0x10 + i as u8);
        wr8(base, 0xFF, 0x20 + i as u8);
        if rd8(base, 0) == 0x10 + i as u8 && rd8(base, 0xFF) == 0x20 + i as u8 {
            pm += 1;
        }
    }
    wr32(BBRAM, 0, BBRAM_SIG);
    wr32(BBRAM, BBRAM_LAST, !BBRAM_SIG);
    let bb = (rd32(BBRAM, 0) == BBRAM_SIG && rd32(BBRAM, BBRAM_LAST) == !BBRAM_SIG) as u32;
    puts(b"mem");
    put_kv(b"code", code);
    put_kv(b"data", data);
    put_kv(b"image", image);
    put_kv(b"pm", pm);
    put_kv(b"bb", bb);
    let mem_ok = code == 1 && data == 1 && image == 1 && pm == 4 && bb == 1;
    puts(if mem_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 21. SPIP.
    let reset = (rd16(SPIP, SPIP_CTL1) == 0 && rd8(SPIP, SPIP_STAT) == 0) as u32;
    wr16(SPIP, SPIP_DATA, 0xA5);
    let off = (rd8(SPIP, SPIP_STAT) & STAT_RBF == 0) as u32;
    wr16(SPIP, SPIP_CTL1, CTL1_SPIEN);
    wr16(SPIP, SPIP_DATA, 0xA5);
    let s8 = rd8(SPIP, SPIP_STAT);
    let d8 = rd16(SPIP, SPIP_DATA);
    let xfer8 = (s8 & STAT_RBF != 0
        && s8 & STAT_BSY == 0
        && d8 == 0
        && rd8(SPIP, SPIP_STAT) & STAT_RBF == 0) as u32;
    wr16(SPIP, SPIP_CTL1, CTL1_SPIEN | CTL1_MOD);
    wr16(SPIP, SPIP_DATA, 0x1234);
    let s16 = rd8(SPIP, SPIP_STAT);
    let d16 = rd16(SPIP, SPIP_DATA);
    let xfer16 = (s16 & STAT_RBF != 0 && d16 == 0 && rd8(SPIP, SPIP_STAT) & STAT_RBF == 0) as u32;
    // SCDV 0x0E, SCIDL, SCM.
    let cfg_val = (0x0E << 9) | (1 << 8) | (1 << 7) | CTL1_SPIEN;
    wr16(SPIP, SPIP_CTL1, cfg_val);
    let cfg = (rd16(SPIP, SPIP_CTL1) == cfg_val) as u32;
    nvic_set(NVIC_ICPR, SPIP_LINE);
    nvic_set(NVIC_ISER, SPIP_LINE);
    wr16(SPIP, SPIP_CTL1, rd16(SPIP, SPIP_CTL1) | CTL1_EIR);
    let quiet = (SPIP_RD_IRQS.load(Ordering::SeqCst) == 0) as u32;
    wr16(SPIP, SPIP_DATA, 0x5A);
    spin(100);
    let rd_irq = SPIP_RD_IRQS.load(Ordering::SeqCst);
    let rd_clear =
        (rd8(SPIP, SPIP_STAT) & STAT_RBF == 0 && rd16(SPIP, SPIP_CTL1) & CTL1_EIR == 0) as u32;
    wr16(SPIP, SPIP_CTL1, rd16(SPIP, SPIP_CTL1) | CTL1_EIW);
    spin(100);
    let wr_irq = SPIP_WR_IRQS.load(Ordering::SeqCst);
    let wr_clear = (rd16(SPIP, SPIP_CTL1) & CTL1_EIW == 0 && !nvic_pending(SPIP_LINE)) as u32;
    nvic_set(NVIC_ICER, SPIP_LINE);
    puts(b"spip");
    put_kv(b"reset", reset);
    put_kv(b"off", off);
    put_kv(b"x8", xfer8);
    put_kv(b"x16", xfer16);
    put_kv(b"cfg", cfg);
    put_kv(b"quiet", quiet);
    put_kv(b"rirq", rd_irq);
    put_kv(b"rclr", rd_clear);
    put_kv(b"wirq", wr_irq);
    put_kv(b"wclr", wr_clear);
    let spip_ok = reset == 1
        && off == 1
        && xfer8 == 1
        && xfer16 == 1
        && cfg == 1
        && quiet == 1
        && rd_irq == 1
        && rd_clear == 1
        && wr_irq == 1
        && wr_clear == 1;
    puts(if spip_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 22. FIU user-mode access to the modeled 1 MiB internal flash medium.
    let fiu_reset = (rd8(FIU, FIU_CTS) == 0x40 && rd8(FIU, FIU_ECTS) == 0x03) as u32;
    let _ = fiu_exec(0x06, None, 0, 0); // WREN
    let write_enabled = (fiu_exec(0x05, None, 0, 1) & 0x02 != 0) as u32; // RDSR1

    let _ = fiu_exec(0x06, None, 0, 0);
    let _ = fiu_exec(0x02, Some(0x100), 0x5A, FIU_A_SIZE | FIU_WRITE | 1); // Page program
    let programmed = (fiu_exec(0x03, Some(0x100), 0, FIU_A_SIZE | 1) == 0x5A) as u32;
    let write_latch_cleared = (fiu_exec(0x05, None, 0, 1) & 0x02 == 0) as u32;

    let _ = fiu_exec(0x06, None, 0, 0);
    let _ = fiu_exec(0x02, Some(0x100), 0xFF, FIU_A_SIZE | FIU_WRITE | 1);
    let one_way = (fiu_exec(0x03, Some(0x100), 0, FIU_A_SIZE | 1) == 0x5A) as u32;

    let _ = fiu_exec(0x06, None, 0, 0);
    let _ = fiu_exec(0xD8, Some(0), 0, FIU_A_SIZE); // Renode-supported 64 KiB erase
    let erased = (fiu_exec(0x03, Some(0x100), 0, FIU_A_SIZE | 1) == 0xFF) as u32;
    let start_cleared = (rd8(FIU, FIU_CTS) & FIU_START == 0) as u32;
    puts(b"fiu");
    put_kv(b"reset", fiu_reset);
    put_kv(b"wel", write_enabled);
    put_kv(b"prog", programmed);
    put_kv(b"clear", write_latch_cleared);
    put_kv(b"oneway", one_way);
    put_kv(b"erase", erased);
    put_kv(b"start", start_cleared);
    let fiu_ok = fiu_reset == 1
        && write_enabled == 1
        && programmed == 1
        && write_latch_cleared == 1
        && one_way == 1
        && erased == 1
        && start_cleared == 1;
    puts(if fiu_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // Expiry: no touch; the watchdog resets the chip on the 10th period.
    puts(b"twd expire ");
    TWD_DOTS.store(1, Ordering::SeqCst);
    wr8(TWD, WDCNT, 10);
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}
