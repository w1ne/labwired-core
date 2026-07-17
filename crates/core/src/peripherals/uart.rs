// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::any::Any;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// Phase 2B.3b (issue #192): the UART uses a single self-perpetuating event
/// token — it has only one kind of wakeup ("do one tick of work"), so the
/// value is arbitrary and never disambiguated in `on_event`.
const UART_WAKE_TOKEN: u32 = 0;
const UART_TRACE_LIMIT: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UartTraceEvent {
    pub seq: u64,
    pub direction: &'static str,
    pub byte: u8,
}

/// A device that emits bytes through the UART's RX path (e.g. a GPS module).
pub trait UartStreamDevice: Send {
    /// Called periodically by the bus tick. Returns the next byte to push into UART RX,
    /// or None if no byte is pending. Implementations should respect `elapsed_us` to
    /// pace output (e.g. 9600 baud → ~1 ms/byte → emit one byte per ~1000 us tick).
    fn poll(&mut self, elapsed_us: u32) -> Option<u8>;
    /// Observe a byte transmitted by firmware on the TX path. Default: ignore.
    /// Bidirectional peers (e.g. an IO-Link master) override this to receive the
    /// device's responses, complementing `poll` which drives the RX path.
    fn on_tx_byte(&mut self, _byte: u8) {}
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    /// Runtime-drivable view of this device, if it accepts simulated input.
    /// Same contract as the hook on `I2cDevice`: input devices override it so
    /// the generic [`crate::Machine::set_input`] resolver can reach them
    /// without a downcast. Default `None` = not an input device.
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UartRegisterLayout {
    #[default]
    Stm32F1,
    Stm32V2,
    Nrf52,
    /// NXP Kinetis Low-Power UART (LPUART), as on the KW41Z / K-series. A flat
    /// 32-bit register block: BAUD@0x00, STAT@0x04 (TDRE/TC/RDRF in the high
    /// byte), CTRL@0x08 (TE/RE + TIE/TCIE interrupt enables), DATA@0x0C
    /// (read pops RX, write transmits). Register offsets ingested from the
    /// public CMSIS-SVD (cmsis-svd-data: NXP/MKW41Z4.svd).
    Lpuart,
    /// Standard 16550 (THR/RBR@0x00, LSR@0x05). Byte-addressed.
    Ns16550,
    /// Synopsys DW_apb_uart: 16550 semantics, 4-byte register stride, LSR@0x14
    /// (Dialog/Renesas DA1469x).
    DwApbUart,
    /// ARM PrimeCell PL011 (DR@0x00, FR@0x18; RXFE set when empty).
    Pl011,
    /// Cadence UART (Xilinx Zynq) — FIFO@0x30, SR@0x2C.
    Cadence,
    /// Silicon Labs EFM32 USART (Series 0) — STATUS@0x10, reset 0x40.
    Efm32,
    /// Silicon Labs EFR32 USART (Series 1) — STATUS@0x10, reset 0x2040.
    Efr32,
    /// Silicon Labs LEUART (Low Energy UART) — STATUS@0x08, reset 0x10.
    Leuart,
    /// Renesas SCI (classic SH/RX and RA-series) — SSR@0x04, byte registers.
    Sci,
    /// Gaisler APBUART (LEON/GRLIB) — DATA@0x00, STATUS@0x04.
    Gaisler,
    /// Nuvoton NPCX — UTBUF@0x00, URBUF@0x02, readiness in UICTRL@0x04.
    Npcx,
    /// Maxim MAX32650 — FIFO@0x1C, STAT@0x08.
    Max32650,
    /// lowRISC OpenTitan — WDATA@0x1C, RDATA@0x18, STATUS@0x14.
    OpenTitan,
    /// Atmel/Microchip SAM USART (SAM3/SAM4) — US_CSR@0x14, US_THR@0x1C.
    Sam,
    /// Microchip SAMD5x/SAME5x SERCOM USART — DATA@0x28, INTFLAG@0x18.
    Sercom,
    /// NXP i.MX UART — UTXD@0x40, URXD@0x00, USR1@0x94.
    Imx,
    /// SiFive UART (FE310) — txdata@0x00 / rxdata@0x04, status folded in data
    /// (TX faithful, RX presence approximate).
    Sifive,
    /// LiteX UART — rxtx@0x00, txfull@0x04 (TX faithful, RX approximate).
    Litex,
    /// VexRiscv Murax SoC UART (SpinalHDL) — DATA@0x00, STATUS@0x04 with TX-free
    /// count in bits[23:16] and RX-occupancy in bits[31:24].
    Murax,
    /// Microsemi/Microchip CoreUARTapb (Mi-V) — TxData@0x00, RxData@0x04,
    /// Status@0x10 (TXRDY bit0, RXRDY bit1).
    CoreUart,
    /// NXP/Freescale Kinetis K6x UART (classic 8-bit, not LPUART) — D@0x07,
    /// S1@0x04 (TDRE bit7, TC bit6, RDRF bit5).
    KinetisUart,
    /// PULP uDMA UART — DMA-only TX (no byte-write register); STATUS@0x20,
    /// DATA@0x34. Estate-level: reads don't fault, TX byte-path is nominal.
    Pulp,
    /// Freescale MPC5500 eSCI (MPC5567) — DR@0x06 (byte at 0x07), SR@0x08
    /// (TDRE bit31, TC bit30, RDRF bit29). Big-endian part; this estate-level
    /// model keeps the engine's little-endian status byte order.
    Esci,
    /// PicoSoC simpleuart — reg_dat@0x04 (write=TX, read=RX), no status register
    /// (TX blocks in HW; RX reads -1 when empty). Estate-level.
    PicoUart,
}

impl FromStr for UartRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32v2" | "v2" | "modern" | "stm32-modern" | "h5" | "stm32h5" => Ok(Self::Stm32V2),
            "nrf52" | "nordic" => Ok(Self::Nrf52),
            "lpuart" | "kinetis" | "nxp" => Ok(Self::Lpuart),
            "16550" | "ns16550" | "uart16550" => Ok(Self::Ns16550),
            "dw_apb_uart" | "dwapb" | "designware" => Ok(Self::DwApbUart),
            "pl011" | "primecell" | "rp2040" => Ok(Self::Pl011),
            "cadence" | "cdns" | "zynq" => Ok(Self::Cadence),
            "efm32" => Ok(Self::Efm32),
            "efr32" => Ok(Self::Efr32),
            "leuart" => Ok(Self::Leuart),
            "sci" | "renesas_sci" | "sh_sci" => Ok(Self::Sci),
            "gaisler" | "apbuart" | "grlib" => Ok(Self::Gaisler),
            "npcx" | "nuvoton" => Ok(Self::Npcx),
            "max32650" | "maxim" => Ok(Self::Max32650),
            "opentitan" | "lowrisc" => Ok(Self::OpenTitan),
            "sam" | "sam_usart" | "atmel" => Ok(Self::Sam),
            "sercom" | "samd5" => Ok(Self::Sercom),
            "imx" | "imxuart" => Ok(Self::Imx),
            "sifive" => Ok(Self::Sifive),
            "litex" => Ok(Self::Litex),
            "murax" => Ok(Self::Murax),
            "coreuart" | "miv" => Ok(Self::CoreUart),
            "k6xf" | "kinetis_uart" => Ok(Self::KinetisUart),
            "pulp" | "udma" => Ok(Self::Pulp),
            "esci" | "mpc5567" => Ok(Self::Esci),
            "picosoc" | "simpleuart" => Ok(Self::PicoUart),
            _ => Err(format!(
                "unsupported UART register layout '{}'; supported: stm32f1, stm32v2, nrf52, \
                 lpuart, ns16550, dw_apb_uart, pl011, cadence, efm32, efr32, leuart, sci, \
                 gaisler, npcx, max32650, opentitan, sam, sercom, imx, sifive, litex, murax, \
                 coreuart, k6xf, pulp",
                value
            )),
        }
    }
}

/// The complete per-family UART register map: register offsets plus the
/// interrupt-enable bit masks. **Every** family difference lives in this one
/// descriptor — the TX sink / RX buffer / stream / scheduler engine on `Uart`
/// is architecture-independent and shared. Adding or changing a family touches
/// only its arm of `regmap`, never another family's.
#[derive(Debug, Clone, Copy)]
struct UartRegMap {
    status: u64,
    tx: u64,
    rx: u64,
    cr3: u64,
    /// CR1 base offset, or `None` for families with no CR1 interrupt concept.
    cr1: Option<u64>,
    txeie_mask: u32,
    tcie_mask: u32,
    /// Width in bytes of the status register window, [`status`, `status`+N).
    /// 4 for 32-bit status words; 1 for byte-register families (16550 LSR,
    /// Renesas SSR, NPCX UICTRL) where the next byte is a *different* register
    /// (e.g. SCI packs SSR@0x04 and RDR@0x05 adjacently).
    status_width: u64,
    /// Status-register value at idle: every TX-ready / transmitter-empty flag
    /// set and the RX shown empty. Any byte read within the window returns the
    /// matching byte of this value (so an 8-bit LSR and a 32-bit STAT share one
    /// path).
    status_idle: u32,
    /// Bits OR-ed into the status word when the RX buffer holds a byte —
    /// active-high "data present" flags (16550 LSR.DR, STM32 RXNE, Kinetis RDRF).
    rx_present_set: u32,
    /// Bits cleared from the status word when the RX buffer holds a byte —
    /// active-high "empty" flags that go low when data arrives (PL011 FR.RXFE,
    /// Cadence SR.RxEMPTY). Most families leave this 0.
    rx_present_clear: u32,
}

impl UartRegisterLayout {
    fn regmap(self) -> UartRegMap {
        match self {
            UartRegisterLayout::Stm32F1 => UartRegMap {
                status: 0x00, // SR
                tx: 0x04,     // DR
                rx: 0x04,     // DR
                cr3: 0x14,
                cr1: Some(0x0C),
                txeie_mask: 1 << 7, // TXEIE
                tcie_mask: 1 << 6,  // TCIE
                status_width: 4,
                status_idle: 0xC0,      // TXE | TC
                rx_present_set: 1 << 5, // RXNE
                rx_present_clear: 0,
            },
            // CR1 bit 3 is TE (transmitter enable) on the v2 USART — NOT an
            // interrupt enable; TXEIE/TXFNFIE lives at bit 7. The mask
            // previously said `1 << 3`, so any firmware that turned the
            // transmitter on held a phantom TX interrupt — invisible until
            // foreign firmware enabled the NVIC line and spun in its default
            // handler. Silicon-pinned on the bench NUCLEO-H563ZI
            // (2026-06-11): CR1=FIFOEN|TE|RE|UE pends nothing; adding TXEIE
            // (bit 7) pends the USART3 NVIC line (TXFNF high at idle).
            UartRegisterLayout::Stm32V2 => UartRegMap {
                status: 0x1C, // ISR
                tx: 0x28,     // TDR
                rx: 0x24,     // RDR
                cr3: 0x08,
                cr1: Some(0x00),
                txeie_mask: 1 << 7, // TXEIE/TXFNFIE
                tcie_mask: 1 << 6,  // TCIE
                status_width: 4,
                status_idle: 0xC0,      // TXE | TC
                rx_present_set: 1 << 5, // RXNE
                rx_present_clear: 0,
            },
            UartRegisterLayout::Nrf52 => UartRegMap {
                status: 0x400, // EVENTS_TXDRDY
                tx: 0x51C,     // TXD
                rx: 0x518,     // RXD
                cr3: 0x500,    // ENABLE
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0xC0,
                rx_present_set: 1 << 5,
                rx_present_clear: 0,
            },
            // NXP Kinetis LPUART. Flat 32-bit block: STAT.TDRE(23)/TC(22)/
            // RDRF(21) → ready flags in byte 2 of the status word; CTRL.TIE(23)
            // / TCIE(22) are the TX interrupt enables; DATA@0x0C is the shared
            // TX/RX data register. CR3 points at MODIR (no DMAT-on-CR3 concept;
            // the smoke path never touches it).
            UartRegisterLayout::Lpuart => UartRegMap {
                status: 0x04,        // STAT
                tx: 0x0C,            // DATA (write transmits)
                rx: 0x0C,            // DATA (read pops RX)
                cr3: 0x14,           // MODIR
                cr1: Some(0x08),     // CTRL
                txeie_mask: 1 << 23, // TIE
                tcie_mask: 1 << 22,  // TCIE
                status_width: 4,
                status_idle: 0x00C0_0000, // TDRE | TC (bits 23/22)
                rx_present_set: 1 << 21,  // RDRF
                rx_present_clear: 0,
            },
            // ── Vendor UART families (register maps from public datasheets /
            //    vendor CMSIS headers / in-tree Linux drivers). All share the
            //    generic TX-sink + status-word engine; only offsets and the
            //    idle/rx flag masks differ. cr3 is parked at an unused offset
            //    (0xF00) for families with no STM32-style DMAT-on-CR3 concept.
            // Standard 16550 (PC16550D). THR/RBR@0x00, LSR@0x05 (8-bit):
            // THRE(5)|TEMT(6) ready, DR(0) set when data present. Reset 0x60.
            UartRegisterLayout::Ns16550 => UartRegMap {
                status: 0x05,
                tx: 0x00,
                rx: 0x00,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 1,
                status_idle: 0x60,
                rx_present_set: 1 << 0, // DR
                rx_present_clear: 0,
            },
            // Synopsys DW_apb_uart (16550 semantics, 4-byte register stride),
            // as on Dialog/Renesas DA1469x. THR/RBR@0x00, LSR@0x14. Reset 0x60.
            UartRegisterLayout::DwApbUart => UartRegMap {
                status: 0x14,
                tx: 0x00,
                rx: 0x00,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x60, // THRE | TEMT
                rx_present_set: 1 << 0,
                rx_present_clear: 0,
            },
            // ARM PrimeCell PL011 (DDI 0183G). DR@0x00, FR@0x18: TXFE(7) ready,
            // RXFE(4) SET WHEN EMPTY (cleared on data), RXFF(6). Reset 0x90.
            UartRegisterLayout::Pl011 => UartRegMap {
                status: 0x18,
                tx: 0x00,
                rx: 0x00,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x90,        // TXFE | RXFE (TX empty, RX empty)
                rx_present_set: 1 << 6,   // RXFF
                rx_present_clear: 1 << 4, // clear RXFE on data
            },
            // Cadence UART (Xilinx Zynq UG585). FIFO@0x30, SR@0x2C: TxEMPTY(3),
            // RxEMPTY(1) SET WHEN EMPTY. Reset 0x0A.
            UartRegisterLayout::Cadence => UartRegMap {
                status: 0x2C,
                tx: 0x30,
                rx: 0x30,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x0A, // TxEMPTY(3) | RxEMPTY(1)
                rx_present_set: 0,
                rx_present_clear: 1 << 1, // clear RxEMPTY on data
            },
            // Silicon Labs EFM32 USART (Series 0). TXDATA@0x34, RXDATA@0x1C,
            // STATUS@0x10: TXBL(6) ready, RXDATAV(7) set on data. Reset 0x40.
            UartRegisterLayout::Efm32 => UartRegMap {
                status: 0x10,
                tx: 0x34,
                rx: 0x1C,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x40,      // TXBL
                rx_present_set: 1 << 7, // RXDATAV
                rx_present_clear: 0,
            },
            // Silicon Labs EFR32 USART (Series 1): same map as EFM32, reset
            // STATUS adds TXIDLE(13) → 0x2040.
            UartRegisterLayout::Efr32 => UartRegMap {
                status: 0x10,
                tx: 0x34,
                rx: 0x1C,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x2040,    // TXBL | TXIDLE
                rx_present_set: 1 << 7, // RXDATAV
                rx_present_clear: 0,
            },
            // Silicon Labs LEUART (Low Energy UART). TXDATA@0x28, RXDATA@0x1C,
            // STATUS@0x08: TXBL(4) ready, RXDATAV(5) set on data. Reset 0x10.
            UartRegisterLayout::Leuart => UartRegMap {
                status: 0x08,
                tx: 0x28,
                rx: 0x1C,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x10,      // TXBL
                rx_present_set: 1 << 5, // RXDATAV
                rx_present_clear: 0,
            },
            // Renesas SCI (classic SH/RX and RA-series, async non-FIFO).
            // TDR@0x03, SSR@0x04 (8-bit): TDRE(7)|TEND(2) ready, RDRF(6) set on
            // data; RDR@0x05. Reset SSR 0x84. status_width=1 so RDR stays clear.
            UartRegisterLayout::Sci => UartRegMap {
                status: 0x04,
                tx: 0x03,
                rx: 0x05,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 1,
                status_idle: 0x84,      // TDRE | TEND
                rx_present_set: 1 << 6, // RDRF
                rx_present_clear: 0,
            },
            // Gaisler APBUART (LEON/GRLIB). DATA@0x00, STATUS@0x04: TS(1)|TE(2)
            // ready, DR(0) set on data. Reset 0x06.
            UartRegisterLayout::Gaisler => UartRegMap {
                status: 0x04,
                tx: 0x00,
                rx: 0x00,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x06,      // TS | TE
                rx_present_set: 1 << 0, // DR
                rx_present_clear: 0,
            },
            // Nuvoton NPCX (Zephyr). UTBUF@0x00 (TX), URBUF@0x02 (RX), readiness
            // in UICTRL@0x04 (8-bit): TBE(0) ready, RBF(1) set on data. The
            // error-only USTAT@0x06 is not modelled (reads 0 = no errors).
            UartRegisterLayout::Npcx => UartRegMap {
                status: 0x04,
                tx: 0x00,
                rx: 0x02,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 1,
                status_idle: 0x01,      // TBE
                rx_present_set: 1 << 1, // RBF
                rx_present_clear: 0,
            },
            // Maxim MAX32650. FIFO@0x1C, STAT@0x08: TX_EMPTY(6) ready (TX_FULL(7)
            // clear), RX_EMPTY(4) SET WHEN EMPTY. Reset 0x50.
            UartRegisterLayout::Max32650 => UartRegMap {
                status: 0x08,
                tx: 0x1C,
                rx: 0x1C,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x50, // RX_EMPTY(4) | TX_EMPTY(6)
                rx_present_set: 0,
                rx_present_clear: 1 << 4, // clear RX_EMPTY on data
            },
            // lowRISC OpenTitan. WDATA@0x1C (TX), RDATA@0x18 (RX), STATUS@0x14:
            // TXFULL(0) clear = ready, TXEMPTY(2)|TXIDLE(3) set, RXEMPTY(5) SET
            // WHEN EMPTY. Reset 0x2C.
            UartRegisterLayout::OpenTitan => UartRegMap {
                status: 0x14,
                tx: 0x1C,
                rx: 0x18,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x2C, // TXEMPTY(2) | TXIDLE(3) | RXEMPTY(5)
                rx_present_set: 0,
                rx_present_clear: 1 << 5, // clear RXEMPTY on data
            },
            // Atmel/Microchip SAM USART (SAM3/SAM4 "US"). US_THR@0x1C, US_RHR@
            // 0x18, US_CSR@0x14: TXRDY(1)|TXEMPTY(9) ready, RXRDY(0) set on data.
            // Idle (TX enabled) 0x202.
            UartRegisterLayout::Sam => UartRegMap {
                status: 0x14,
                tx: 0x1C,
                rx: 0x18,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x202,     // TXRDY | TXEMPTY
                rx_present_set: 1 << 0, // RXRDY
                rx_present_clear: 0,
            },
            // Microchip SAMD5x/SAME5x SERCOM USART. Shared DATA@0x28, INTFLAG@
            // 0x18 (8-bit): DRE(0) ready, RXC(2) set on data. Idle 0x01.
            UartRegisterLayout::Sercom => UartRegMap {
                status: 0x18,
                tx: 0x28,
                rx: 0x28,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 1,
                status_idle: 0x01,      // DRE
                rx_present_set: 1 << 2, // RXC
                rx_present_clear: 0,
            },
            // NXP i.MX UART. UTXD@0x40, URXD@0x00, USR1@0x94: TRDY(13) ready,
            // RRDY(9) set on data. Idle USR1 0x2040 (USR2@0x98 not modelled).
            UartRegisterLayout::Imx => UartRegMap {
                status: 0x94,
                tx: 0x40,
                rx: 0x00,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x2040,    // TRDY | RXDS
                rx_present_set: 1 << 9, // RRDY
                rx_present_clear: 0,
            },
            // SiFive UART (FE310). txdata@0x00 (bit31 full), rxdata@0x04 (bit31
            // empty). Status is folded into the data registers, so the generic
            // engine models TX faithfully (write @0x00 transmits; full reads 0 =
            // ready) but RX presence (rxdata bit31) is approximate.
            UartRegisterLayout::Sifive => UartRegMap {
                status: 0x00, // txdata.full = 0 at idle (ready)
                tx: 0x00,
                rx: 0x04,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x0000_0000,
                rx_present_set: 0,
                rx_present_clear: 0,
            },
            // LiteX UART. rxtx@0x00 (shared), txfull@0x04, rxempty@0x08. The
            // generic engine polls txfull@0x04 (idle 0 = ready) for faithful TX;
            // rxempty@0x08 is not in the window, so RX presence is approximate.
            UartRegisterLayout::Litex => UartRegMap {
                status: 0x04, // txfull = 0 at idle (ready)
                tx: 0x00,
                rx: 0x00,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x0000_0000,
                rx_present_set: 0,
                rx_present_clear: 0,
            },
            // VexRiscv Murax. DATA@0x00, STATUS@0x04: TX-free count bits[23:16]
            // (idle 16 = 0x100000, firmware writes while != 0), RX-occupancy
            // bits[31:24] (!= 0 means data present).
            UartRegisterLayout::Murax => UartRegMap {
                status: 0x04,
                tx: 0x00,
                rx: 0x00,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x0010_0000,    // TX FIFO free = 16
                rx_present_set: 0x0100_0000, // RX occupancy = 1
                rx_present_clear: 0,
            },
            // Microsemi CoreUARTapb. TxData@0x00, RxData@0x04, Status@0x10:
            // TXRDY(0) ready, RXRDY(1) set on data. Idle 0x01.
            UartRegisterLayout::CoreUart => UartRegMap {
                status: 0x10,
                tx: 0x00,
                rx: 0x04,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x01,      // TXRDY
                rx_present_set: 1 << 1, // RXRDY
                rx_present_clear: 0,
            },
            // NXP Kinetis K6x UART (classic). D@0x07, S1@0x04 (8-bit): TDRE(7)|
            // TC(6) ready, RDRF(5) set on data. Idle 0xC0. width=1 (S2@0x05).
            UartRegisterLayout::KinetisUart => UartRegMap {
                status: 0x04,
                tx: 0x07,
                rx: 0x07,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 1,
                status_idle: 0xC0,      // TDRE | TC
                rx_present_set: 1 << 5, // RDRF
                rx_present_clear: 0,
            },
            // PULP uDMA UART. Transmit is DMA-descriptor only on real silicon —
            // there is no byte-write TX register — so this is an estate-level
            // model: STATUS@0x20 (TX_BUSY=0 idle), DATA@0x34 read. RX presence
            // (VALID@0x30) is not in the status window, so it is approximate.
            UartRegisterLayout::Pulp => UartRegMap {
                status: 0x20,
                tx: 0x34,
                rx: 0x34,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x0000_0000, // TX_BUSY = 0 (ready)
                rx_present_set: 0,
                rx_present_clear: 0,
            },
            // Freescale MPC5567 eSCI. DR@0x06 (data byte at 0x07), SR@0x08:
            // TDRE(31)|TC(30) ready, RDRF(29) set on data. Idle SR 0xC0000000.
            UartRegisterLayout::Esci => UartRegMap {
                status: 0x08,
                tx: 0x07,
                rx: 0x07,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0xC000_0000,    // TDRE | TC
                rx_present_set: 0x2000_0000, // RDRF
                rx_present_clear: 0,
            },
            // PicoSoC simpleuart. reg_dat@0x04 (write=TX, read=RX). No status
            // register exists (TX blocks in HW); status parks at reg_div@0x00.
            UartRegisterLayout::PicoUart => UartRegMap {
                status: 0x00,
                tx: 0x04,
                rx: 0x04,
                cr3: 0xF00,
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
                status_width: 4,
                status_idle: 0x0000_0000,
                rx_present_set: 0,
                rx_present_clear: 0,
            },
        }
    }
}

/// Minimal UART mock with selectable register layout.
#[derive(serde::Serialize)]
pub struct Uart {
    layout: UartRegisterLayout,
    #[serde(skip)]
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    #[serde(skip)]
    rx_buf: Arc<Mutex<VecDeque<u8>>>,
    echo_stdout: bool,
    /// Optional prefix emitted at the start of each echoed line (used to label
    /// per-device output when several machines share one stdout, e.g. the
    /// two-C3 WiFi run). `None` = no prefix.
    #[serde(skip)]
    stdout_prefix: Option<String>,
    /// Per-line accumulator used when `stdout_prefix` is set: bytes buffer here
    /// until a newline, then the whole `prefix + line` is printed in one call —
    /// so two machines sharing stdout produce atomic, non-interleaved lines.
    #[serde(skip)]
    stdout_line_buf: String,
    /// CR1 register (tracks TXEIE and TE bits for interrupt-driven TX simulation).
    cr1: u32,
    cr3: u32,
    /// F1-layout config registers, captured for read-back fidelity (BRR/CR2/GTPR
    /// have no behavioural effect in this instruction-level model, but firmware
    /// reads them back). Masked to the silicon writable bits on read. Unused by
    /// the V2 layout, whose register map differs.
    cr2: u32,
    brr: u32,
    gtpr: u32,
    /// CR3 writable mask — a per-part delta on the shared F1 USART map. The F1
    /// USART implements bits [10:0] (`0x07FF`); the F4 USART adds bit 11
    /// (ONEBIT, one-sample-bit mode) → `0x0FFF`, silicon-confirmed on the bench
    /// F103 (0x07FF) and F407 (0x0FFF). Set from the chip config's `cr3_mask`.
    cr3_mask: u32,
    dma_tx_pending: bool,
    /// Stream devices attached to the RX path (e.g. GPS modules).
    #[serde(skip)]
    pub attached_streams: Vec<Box<dyn UartStreamDevice>>,
    #[serde(skip)]
    trace: VecDeque<UartTraceEvent>,
    #[serde(skip)]
    trace_seq: u64,
    /// Microseconds accumulated since last stream tick.
    elapsed_us: u32,
    /// Phase 2B.3b (issue #192): whether a self-perpetuating scheduler WAKE
    /// event is currently in flight. Guards against double-arming. Only used
    /// under the `event-scheduler` feature (flag-off drives via `tick()`).
    #[serde(skip)]
    scheduled: bool,
    /// Whether this UART was registered with an IRQ line (`PeripheralEntry::irq`).
    /// Set at the bus attach choke points via `attach_irq_line`.
    ///
    /// `true` by DEFAULT — the conservative value. `has_active_work` uses it to
    /// decide whether holding a level-triggered TXEIE/TCIE is worth a per-cycle
    /// wakeup: the machine drops `raise_own_irq` when the entry has no `irq`, so
    /// on such a bus (e.g. the ESP32-C3, whose chip yaml declares `uart0` with no
    /// `irq:` and whose intmatrix never reads this model — it implements no
    /// `matrix_irq_sources`) those wakeups are provably unobservable. Defaulting
    /// to `true` means any bus that bypasses the choke points keeps the exact
    /// legacy cadence.
    ///
    /// ⚠️ `true`-by-default holds only because the CONSTRUCTOR sets it. `Uart`
    /// derives `Serialize` ALONE — there is no `Deserialize`. If you ever add
    /// one, `#[serde(skip)]` will populate this from `bool::default()` == FALSE,
    /// which is the UNSAFE direction: a restored UART would silently stop
    /// scheduling real IRQ wakeups on buses that DO wire an IRQ (STM32 et al),
    /// dropping interrupts with no error. Adding `Deserialize` REQUIRES
    /// `#[serde(skip, default = "…")]` returning `true`. Stale `true` is merely
    /// slow; stale `false` breaks fidelity.
    #[serde(skip)]
    irq_wired: bool,
    /// Test/differential knob: pin this model to the legacy per-cycle walk
    /// (`uses_scheduler() == false`). `false` in every real config. See
    /// [`Uart::force_legacy_walk`].
    #[serde(skip)]
    legacy_walk_forced: bool,
}

impl core::fmt::Debug for Uart {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Uart")
            .field("layout", &self.layout)
            .field("cr1", &self.cr1)
            .field("streams", &self.attached_streams.len())
            .finish()
    }
}

impl Default for Uart {
    fn default() -> Self {
        Self::new()
    }
}

impl Uart {
    pub fn new() -> Self {
        Self::new_with_layout(UartRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: UartRegisterLayout) -> Self {
        Self::new_with_layout_cr3(layout, 0x0000_07FF)
    }

    /// Like [`new_with_layout`] but with an explicit CR3 writable mask — the
    /// per-part delta on the shared F1 map (F1 `0x07FF`, F4 `0x0FFF`).
    pub fn new_with_layout_cr3(layout: UartRegisterLayout, cr3_mask: u32) -> Self {
        Self {
            layout,
            sink: None,
            rx_buf: Arc::new(Mutex::new(VecDeque::new())),
            echo_stdout: true,
            stdout_prefix: None,
            stdout_line_buf: String::new(),
            cr1: 0,
            cr3: 0,
            cr2: 0,
            brr: 0,
            gtpr: 0,
            cr3_mask,
            dma_tx_pending: false,
            attached_streams: Vec::new(),
            trace: VecDeque::new(),
            trace_seq: 0,
            elapsed_us: 0,
            scheduled: false,
            // Conservative until the bus says otherwise (see `attach_irq_line`).
            irq_wired: true,
            legacy_walk_forced: false,
        }
    }

    /// Test/differential knob: pin the UART to the legacy per-cycle walk
    /// (`uses_scheduler() == false`), so the bus drives it through `tick()`
    /// every cycle — the exact pre-scheduler semantics. Used by
    /// `esp32c3_walk_differential` to build the ground-truth reference config
    /// from the same bus assembly (mirrors `Esp32c3I2c::force_legacy_walk`).
    ///
    /// This is the reference the `has_active_work` IRQ gate is proven against:
    /// the walk ticks this model every cycle unconditionally, so if skipping
    /// unobservable scheduler wakeups changed ANY guest-visible byte, the
    /// walk-on-vs-scheduler differential would diverge.
    pub fn force_legacy_walk(&mut self) {
        self.legacy_walk_forced = true;
    }

    /// Attach a stream device to the UART RX path.
    pub fn attach_stream(&mut self, dev: Box<dyn UartStreamDevice>) {
        self.attached_streams.push(dev);
    }

    /// Get a shared handle to the RX buffer for external data injection.
    pub fn rx_buffer(&self) -> Arc<Mutex<VecDeque<u8>>> {
        self.rx_buf.clone()
    }

    /// Phase 2B.3b (issue #192): does the UART have anything that needs a
    /// per-tick wakeup? Level-triggered TXEIE/TCIE, an attached RX stream, or
    /// a pending DMA TX. Drives both the initial scheduler arm and the
    /// self-reschedule decision so the event path matches the legacy `tick()`.
    fn has_active_work(&self) -> bool {
        let txeie_set = (self.cr1 & self.txeie_mask()) != 0 && self.txeie_mask() != 0;
        let tcie_set = (self.cr1 & self.tcie_mask()) != 0 && self.tcie_mask() != 0;
        // The TXEIE/TCIE arm's ONLY product is `raise_own_irq` — `advance_one_tick`
        // feeds it nowhere else, and it mutates no state on this path (the
        // `elapsed_us`/stream bookkeeping lives behind the `attached_streams`
        // arm, the DMA signal behind `dma_tx_pending`). The machine DROPS
        // `raise_own_irq` when the entry carries no `irq` (`apply_event_result`),
        // so on an IRQ-less bus a wakeup held open by TXEIE/TCIE alone can
        // neither pend a line nor change a byte: it is unobservable work. Skip
        // scheduling it there, and keep the exact legacy cadence everywhere the
        // line is actually wired (`irq_wired` defaults to `true`).
        //
        // This narrows WAKEUPS only. Event DELIVERY is untouched: the moment an
        // MMIO write gives the UART real work (a stream byte to pace, a DMA TX),
        // `take_scheduled_events` re-arms at the same cycle it always did.
        let level_irq_observable = self.irq_wired && (txeie_set || tcie_set);
        level_irq_observable || !self.attached_streams.is_empty() || self.dma_tx_pending
    }

    /// Phase 2B.3b: one tick-equivalent of work, shared verbatim by the legacy
    /// `tick()` and the scheduler `on_event` so both paths are identical.
    /// Returns `(raise_irq, dma_signals)`.
    fn advance_one_tick(&mut self) -> (bool, Vec<u32>) {
        let mut dma_signals = Vec::new();
        if self.dma_tx_pending {
            dma_signals.push(1); // 1 = TX Signal
            self.dma_tx_pending = false;
        }

        // Poll attached stream devices and push emitted bytes into the RX
        // buffer. Each tick represents ~1000 µs (1 ms) of simulated time. At
        // 9600 baud that is about 1 byte/ms, which matches the GPS pacing.
        if !self.attached_streams.is_empty() {
            const TICK_US: u32 = 1000;
            self.elapsed_us = self.elapsed_us.saturating_add(TICK_US);
            let elapsed = self.elapsed_us;
            self.elapsed_us = 0; // consumed this tick

            let rx_trace = if let Ok(mut rx_guard) = self.rx_buf.lock() {
                let mut rx_trace = Vec::new();
                for stream in &mut self.attached_streams {
                    if let Some(byte) = stream.poll(elapsed) {
                        rx_guard.push_back(byte);
                        rx_trace.push(byte);
                    }
                }
                rx_trace
            } else {
                Vec::new()
            };
            for byte in rx_trace {
                self.record_trace("rx", byte);
            }
        }

        // Fire while either TXEIE or TCIE is set:
        // - TXEIE lets HAL push bytes into DR
        // - TCIE delivers the final completion interrupt after the last byte
        let txeie_set = (self.cr1 & self.txeie_mask()) != 0 && self.txeie_mask() != 0;
        let tcie_set = (self.cr1 & self.tcie_mask()) != 0 && self.tcie_mask() != 0;
        (txeie_set || tcie_set, dma_signals)
    }

    // The 7 accessors below all read from the single per-family `regmap()`
    // descriptor, so a family's register map lives in exactly one place.
    fn status_offset(&self) -> u64 {
        self.layout.regmap().status
    }
    /// The status register as a 32-bit word: the idle pattern adjusted for a
    /// pending RX byte (set the active-high "data present" flags, clear the
    /// active-high "empty" flags). Byte-addressed reads slice this word, so an
    /// 8-bit LSR and a 32-bit STAT share one path.
    fn status_word(&self, rx_present: bool) -> u32 {
        let m = self.layout.regmap();
        let mut v = if rx_present {
            (m.status_idle & !m.rx_present_clear) | m.rx_present_set
        } else {
            m.status_idle
        };
        // Modern STM32 USART (ISR at 0x1C): bits 21/22 are TEACK/REACK — hardware
        // sets them once the transmitter/receiver are acknowledged after the USART
        // is enabled (UE) with TE/RE, and Zephyr's uart_stm32_init spins on TEACK.
        // They are 0 while the USART is disabled, so a reset-state ISR read still
        // sees status_idle (0xC0). The legacy SR has no such bits.
        if matches!(self.layout, UartRegisterLayout::Stm32V2) {
            const UE: u32 = 1 << 0;
            const RE: u32 = 1 << 2;
            const TE: u32 = 1 << 3;
            if self.cr1 & UE != 0 {
                if self.cr1 & TE != 0 {
                    v |= 1 << 21; // TEACK
                }
                if self.cr1 & RE != 0 {
                    v |= 1 << 22; // REACK
                }
            }
        }
        v
    }
    fn tx_offset(&self) -> u64 {
        self.layout.regmap().tx
    }
    fn rx_offset(&self) -> u64 {
        self.layout.regmap().rx
    }
    fn cr3_offset(&self) -> u64 {
        self.layout.regmap().cr3
    }

    /// F1-layout config registers as `(base offset, silicon writable mask)`.
    /// Masks silicon-confirmed on the bench F103 (RM0008 §27.6): BRR 0xFFFF,
    /// CR1 0x3FFD, CR2 0x7F6F, CR3 0x07FF, GTPR 0xFFFF.
    const F1_CONFIG: [(u64, u32); 5] = [
        (0x08, 0x0000_FFFF), // BRR
        (0x0C, 0x0000_3FFD), // CR1
        (0x10, 0x0000_7F6F), // CR2
        (0x14, 0x0000_07FF), // CR3
        (0x18, 0x0000_FFFF), // GTPR
    ];

    fn f1_config_value(&self, base: u64) -> u32 {
        match base {
            0x08 => self.brr,
            0x0C => self.cr1,
            0x10 => self.cr2,
            0x14 => self.cr3,
            0x18 => self.gtpr,
            _ => 0,
        }
    }

    /// Masked read-back byte for an F1 config register, or `None` if `offset` is
    /// not one. F1 layout only. CR3 uses the per-part `cr3_mask` (F1 0x07FF / F4
    /// 0x0FFF) rather than the const, since it's the one register that differs.
    fn f1_config_byte(&self, offset: u64) -> Option<u8> {
        for (base, const_mask) in Self::F1_CONFIG {
            let bo = offset.wrapping_sub(base);
            if bo < 4 {
                let mask = if base == 0x14 {
                    self.cr3_mask
                } else {
                    const_mask
                };
                return Some((((self.f1_config_value(base) & mask) >> (bo * 8)) & 0xFF) as u8);
            }
        }
        None
    }

    /// Accumulate one written byte into an F1 config register. Returns true if
    /// `offset` belonged to one. F1 layout only.
    fn f1_config_write(&mut self, offset: u64, value: u8) -> bool {
        for (base, _mask) in Self::F1_CONFIG {
            let bo = offset.wrapping_sub(base);
            if bo < 4 {
                let shift = bo * 8;
                let set =
                    |reg: &mut u32| *reg = (*reg & !(0xFF << shift)) | ((value as u32) << shift);
                match base {
                    0x08 => set(&mut self.brr),
                    0x0C => set(&mut self.cr1),
                    0x10 => set(&mut self.cr2),
                    0x14 => set(&mut self.cr3),
                    0x18 => set(&mut self.gtpr),
                    _ => {}
                }
                return true;
            }
        }
        false
    }
    /// Offset of the CR1 register. `None` for layouts without a CR1 interrupt concept.
    fn cr1_offset(&self) -> Option<u64> {
        self.layout.regmap().cr1
    }
    /// Bitmask of the TXEIE bit within CR1 for interrupt-driven TX detection.
    fn txeie_mask(&self) -> u32 {
        self.layout.regmap().txeie_mask
    }
    /// Bitmask of the transmission-complete interrupt enable bit within CR1.
    fn tcie_mask(&self) -> u32 {
        self.layout.regmap().tcie_mask
    }

    fn push_tx(&mut self, value: u8) {
        self.record_trace("tx", value);

        if let Some(sink) = &self.sink {
            if let Ok(mut guard) = sink.lock() {
                guard.push(value);
            }
        }

        for stream in &mut self.attached_streams {
            stream.on_tx_byte(value);
        }

        if self.echo_stdout {
            #[allow(unused_must_use)]
            if let Some(prefix) = &self.stdout_prefix {
                // Line-buffer so each machine's lines print atomically (no
                // byte-level interleaving when several machines share stdout).
                if value == b'\n' {
                    println!("{prefix}{}", self.stdout_line_buf);
                    self.stdout_line_buf.clear();
                    io::stdout().flush();
                } else if value != b'\r' {
                    self.stdout_line_buf.push(value as char);
                }
            } else {
                print!("{}", value as char);
                io::stdout().flush();
            }
        }
    }

    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }

    fn record_trace(&mut self, direction: &'static str, byte: u8) {
        self.trace_seq = self.trace_seq.wrapping_add(1);
        if self.trace.len() >= UART_TRACE_LIMIT {
            self.trace.pop_front();
        }
        self.trace.push_back(UartTraceEvent {
            seq: self.trace_seq,
            direction,
            byte,
        });
    }

    pub fn trace_snapshot(&self) -> Vec<UartTraceEvent> {
        self.trace.iter().cloned().collect()
    }

    /// Set a prefix emitted before each echoed stdout line, to label this UART's
    /// output when multiple machines share one stdout (e.g. the two-C3 WiFi run).
    pub fn set_stdout_prefix(&mut self, prefix: impl Into<String>) {
        self.stdout_prefix = Some(prefix.into());
    }
}

impl crate::Peripheral for Uart {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let status = self.status_offset();
        if offset >= status && offset < status + self.layout.regmap().status_width {
            let rx_present = self.rx_buf.lock().map(|g| !g.is_empty()).unwrap_or(false);
            let word = self.status_word(rx_present);
            return Ok(((word >> ((offset - status) * 8)) & 0xFF) as u8);
        }
        if offset == self.rx_offset() {
            // Pop one byte from RX buffer
            if let Ok(mut guard) = self.rx_buf.lock() {
                return Ok(guard.pop_front().unwrap_or(0x00));
            }
            return Ok(0x00);
        }
        // F1 config registers (BRR/CR1/CR2/CR3/GTPR), masked read-back.
        if matches!(self.layout, UartRegisterLayout::Stm32F1) {
            if let Some(b) = self.f1_config_byte(offset) {
                return Ok(b);
            }
        }
        // V2 (USARTv2: L4/F7/G0/H7…) BRR read-back. The USART exposes USARTDIV
        // at 0x0C, and Zephyr's uart_stm32_set_baudrate writes it then reads it
        // back to `__ASSERT(BRR >= 16)`. The divisor has no behavioural effect in
        // this instruction-level model (byte timing is not simulated), but the
        // register must read what firmware wrote or the assert panics at boot.
        if matches!(self.layout, UartRegisterLayout::Stm32V2) {
            let bo = offset.wrapping_sub(0x0C);
            if bo < 4 {
                return Ok((((self.brr & 0x0000_FFFF) >> (bo * 8)) & 0xFF) as u8);
            }
        }
        if offset == self.cr3_offset() {
            return Ok(self.cr3 as u8);
        }
        // Return CR1 bytes so interrupt-driven firmware can read back TXEIE state.
        if let Some(cr1_base) = self.cr1_offset() {
            let byte_offset = offset.wrapping_sub(cr1_base);
            if byte_offset < 4 {
                return Ok(((self.cr1 >> (byte_offset * 8)) & 0xFF) as u8);
            }
        }
        Ok(0)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let is_legacy_tx_alias =
            matches!(self.layout, UartRegisterLayout::Stm32F1) && offset == 0x00;

        if offset == self.tx_offset() || is_legacy_tx_alias {
            self.push_tx(value);
            // If DMAT bit is set, we might be in a DMA sequence.
            if (self.cr3 & (1 << 7)) != 0 {
                self.dma_tx_pending = true;
            }
        } else if matches!(self.layout, UartRegisterLayout::Stm32F1)
            && self.f1_config_write(offset, value)
        {
            // F1 config register (BRR/CR1/CR2/CR3/GTPR) captured. CR3.DMAT
            // (bit 7) still gates DMA-driven TX.
            if (self.cr3 & (1 << 7)) != 0 {
                self.dma_tx_pending = true;
            }
        } else if matches!(self.layout, UartRegisterLayout::Stm32V2)
            && offset.wrapping_sub(0x0C) < 4
        {
            // V2 BRR@0x0C: capture the written USARTDIV byte so the driver's
            // read-back assert (BRR >= 16) sees what it wrote. Read-back only.
            let shift = (offset - 0x0C) * 8;
            self.brr = (self.brr & !(0xFF << shift)) | ((value as u32) << shift);
        } else if offset == self.cr3_offset() {
            self.cr3 = value as u32;
            if (self.cr3 & (1 << 7)) != 0 {
                self.dma_tx_pending = true;
            }
        } else if let Some(cr1_base) = self.cr1_offset() {
            // Track CR1 byte-by-byte so TXEIE state is visible to tick().
            let byte_offset = offset.wrapping_sub(cr1_base);
            if byte_offset < 4 {
                let shift = byte_offset * 8;
                self.cr1 = (self.cr1 & !(0xFF << shift)) | ((value as u32) << shift);
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let (irq, dma_signals) = self.advance_one_tick();
        crate::PeripheralTickResult {
            irq,
            dma_signals: (!dma_signals.is_empty()).then_some(dma_signals),
            ..Default::default()
        }
    }

    /// Phase 2B.3b (issue #192): the shared `Uart` is migrated to the event
    /// scheduler. With the feature on, the bus stops calling `tick()` every
    /// cycle; `take_scheduled_events` / `on_event` drive it instead. With the
    /// feature off this is ignored and `tick()` still runs.
    fn uses_scheduler(&self) -> bool {
        !self.legacy_walk_forced
    }

    /// Record whether a line was wired, so `has_active_work` can tell a
    /// level-triggered own-IRQ that someone observes from one that the machine
    /// will drop on the floor. See the field docs on `irq_wired`.
    fn attach_irq_line(&mut self, irq: Option<u32>) {
        self.irq_wired = irq.is_some();
    }

    /// Hand the bus a single self-perpetuating WAKE event when the UART has
    /// active work and none is already in flight. Called after an MMIO write
    /// (TXEIE/TCIE arm, DMA trigger) and once at scheduler bootstrap (so an
    /// RX stream attached before firmware runs gets polled).
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.has_active_work() && !self.scheduled {
            self.scheduled = true;
            vec![(0, UART_WAKE_TOKEN)]
        } else {
            Vec::new()
        }
    }

    /// Fire one tick-equivalent of work and re-arm while there's still work.
    /// `raise_own_irq` mirrors the legacy `tick()` returning `irq: true` (the
    /// bus pends the UART's configured NVIC line); `dma_signals` route exactly
    /// as the legacy path; `reschedule_delay` keeps the level-triggered IRQ
    /// (and stream pacing) going at one event per tick until work drains.
    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let (irq, dma_signals) = self.advance_one_tick();
        let keep_going = self.has_active_work();
        self.scheduled = keep_going;
        crate::sched::EventResult {
            raise_own_irq: irq,
            dma_signals,
            reschedule_delay: keep_going.then_some(1),
            ..Default::default()
        }
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let status = self.status_offset();
        if offset >= status && offset < status + self.layout.regmap().status_width {
            let rx_present = self.rx_buf.lock().map(|g| !g.is_empty()).unwrap_or(false);
            let word = self.status_word(rx_present);
            return Some(((word >> ((offset - status) * 8)) & 0xFF) as u8);
        }
        if offset == self.rx_offset() {
            // Peek without consuming
            if let Ok(guard) = self.rx_buf.lock() {
                return Some(*guard.front().unwrap_or(&0x00));
            }
            return Some(0x00);
        }
        if matches!(self.layout, UartRegisterLayout::Stm32F1) {
            if let Some(b) = self.f1_config_byte(offset) {
                return Some(b);
            }
        }
        if offset == self.cr3_offset() {
            return Some(self.cr3 as u8);
        }
        if let Some(cr1_base) = self.cr1_offset() {
            let byte_offset = offset.wrapping_sub(cr1_base);
            if byte_offset < 4 {
                return Some(((self.cr1 >> (byte_offset * 8)) & 0xFF) as u8);
            }
        }
        Some(0)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::{Uart, UartRegisterLayout};
    use crate::Peripheral;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_uart_f1_transmit_offsets() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);

        // DR offset
        uart.write(0x04, b'A').unwrap();
        // Legacy alias for compatibility in existing fixtures
        uart.write(0x00, b'B').unwrap();

        let data = sink.lock().unwrap().clone();
        assert_eq!(data, vec![b'A', b'B']);
    }

    #[test]
    fn test_uart_v2_transmit_uses_tdr_only() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);

        // Wrong offset for v2 should not transmit.
        uart.write(0x04, b'X').unwrap();
        // TDR offset
        uart.write(0x28, b'Y').unwrap();

        let data = sink.lock().unwrap().clone();
        assert_eq!(data, vec![b'Y']);
        assert_eq!(uart.read(0x1C).unwrap(), 0xC0); // ISR ready flags
    }

    /// Modern-USART ISR exposes TEACK (bit 21) / REACK (bit 22), but ONLY once
    /// the USART is enabled (CR1.UE) with TE/RE set — Zephyr's uart_stm32_init
    /// spins on TEACK after enabling, while firmware that samples ISR at reset
    /// (CONFIG read-back) must still see 0xC0. Word reads must serve the upper
    /// bytes too, since the driver reads ISR with a 32-bit load.
    #[test]
    fn test_uart_v2_teack_gated_on_enable() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
        // Reset: UART disabled → no TEACK/REACK.
        assert_eq!(uart.read_u32(0x1C).unwrap(), 0xC0);
        // CR1 (offset 0x00 on v2) = UE | TE | RE = 0xD.
        uart.write(0x00, 0x0D).unwrap();
        let isr = uart.read_u32(0x1C).unwrap();
        assert_ne!(isr & (1 << 21), 0, "TEACK set once UE+TE");
        assert_ne!(isr & (1 << 22), 0, "REACK set once UE+RE");
        assert_eq!(isr & 0xC0, 0xC0, "TXE/TC still present");
    }

    /// Modern-USART BRR (USARTDIV) lives at 0x0C and must read back what
    /// firmware wrote: Zephyr's uart_stm32_set_baudrate writes BRR then
    /// `__ASSERT(BRR >= 16)`. A model that dropped the write returned 0 and
    /// panicked the kernel before the console banner (silent boot hang).
    /// BRR is read-back-only here — it has no behavioural effect on the
    /// instruction-level TX path (byte timing is not simulated).
    #[test]
    fn test_uart_v2_brr_readback() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);

        // Reset BRR reads 0; the driver's assert would fire here.
        assert_eq!(uart.read_u32(0x0C).unwrap(), 0);

        // 80 MHz PCLK1 / 115200 baud, oversampling 16 → USARTDIV = 694 (0x2B6).
        uart.write_u32(0x0C, 0x2B6).unwrap();
        assert_eq!(uart.read_u32(0x0C).unwrap(), 0x2B6, "BRR reads back");
        assert!(
            uart.read_u32(0x0C).unwrap() >= 16,
            "passes BRR >= 16 assert"
        );

        // Writing BRR must not transmit (TDR is 0x28).
        assert!(sink.lock().unwrap().is_empty(), "BRR write is not a TX");
        // Only the upper 16 bits are reserved; the divisor is 16-bit.
        uart.write_u32(0x0C, 0xFFFF_FFFF).unwrap();
        assert_eq!(uart.read_u32(0x0C).unwrap(), 0x0000_FFFF, "BRR is 16-bit");
    }

    #[test]
    fn test_uart_lpuart_transmit_uses_data_register() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Lpuart);
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);

        // STAT (0x04) must report TDRE (bit 23) + TC (bit 22) ready: that is
        // 0xC0 in byte 2 of the word, i.e. read at offset 0x06.
        assert_eq!(uart.read(0x06).unwrap(), 0xC0, "STAT byte 2 = TDRE|TC");
        // A write to a wrong offset (STAT) must not transmit.
        uart.write(0x04, b'X').unwrap();
        // DATA register at 0x0C transmits.
        uart.write(0x0C, b'K').unwrap();

        assert_eq!(sink.lock().unwrap().clone(), vec![b'K']);
    }

    /// Every vendor layout: a write to its TX data offset reaches the sink, and
    /// its status offset reports transmitter-ready at idle. Offsets are the
    /// datasheet values encoded in `regmap()`.
    #[test]
    fn test_vendor_layout_tx_and_status() {
        use super::UartRegisterLayout::*;
        // (layout, tx_offset, status_offset)
        let cases = [
            (Ns16550, 0x00u64, 0x05u64),
            (DwApbUart, 0x00, 0x14),
            (Pl011, 0x00, 0x18),
            (Cadence, 0x30, 0x2C),
            (Efm32, 0x34, 0x10),
            (Efr32, 0x34, 0x10),
            (Leuart, 0x28, 0x08),
            (Sci, 0x03, 0x04),
            (Gaisler, 0x00, 0x04),
            (Npcx, 0x00, 0x04),
            (Max32650, 0x1C, 0x08),
            (OpenTitan, 0x1C, 0x14),
            (Sam, 0x1C, 0x14),
            (Sercom, 0x28, 0x18),
            (Imx, 0x40, 0x94),
            (Sifive, 0x00, 0x00),
            (Litex, 0x00, 0x04),
            (Murax, 0x00, 0x04),
            (CoreUart, 0x00, 0x10),
            (KinetisUart, 0x07, 0x04),
            (Pulp, 0x34, 0x20),
            (Esci, 0x07, 0x08),
            (PicoUart, 0x04, 0x00),
        ];
        for (layout, tx, status) in cases {
            let mut uart = Uart::new_with_layout(layout);
            let sink = Arc::new(Mutex::new(Vec::new()));
            uart.set_sink(Some(sink.clone()), false);
            uart.write(tx, b'Q').unwrap();
            assert_eq!(
                sink.lock().unwrap().clone(),
                vec![b'Q'],
                "{layout:?}: write to tx offset {tx:#x} must reach the sink"
            );
            // The status register must read non-faulting and is consistent with
            // its idle word (a 32-bit read assembled from the byte path).
            let w = uart.read_u32(status).unwrap();
            assert_eq!(
                w,
                layout.regmap().status_idle,
                "{layout:?}: idle status at {status:#x}"
            );
        }
    }

    /// Empty-flag families (PL011 FR.RXFE, Cadence SR.RxEMPTY, OpenTitan
    /// STATUS.RXEMPTY) must show "RX empty" at idle and clear that bit when a
    /// byte arrives — the inverse of the STM32 "set RXNE" convention.
    #[test]
    fn test_vendor_empty_flag_rx_semantics() {
        use super::UartRegisterLayout::*;
        // (layout, status_offset, rx_empty_bit)
        for (layout, status, empty_bit) in [
            (Pl011, 0x18u64, 4u32),
            (Cadence, 0x2C, 1),
            (OpenTitan, 0x14, 5),
        ] {
            let mut uart = Uart::new_with_layout(layout);
            uart.set_sink(None, false);
            // Idle: empty bit set.
            assert_ne!(
                uart.read_u32(status).unwrap() & (1 << empty_bit),
                0,
                "{layout:?}: RX-empty must be set at idle"
            );
            // A pending RX byte clears the empty bit.
            uart.rx_buffer().lock().unwrap().push_back(b'Z');
            assert_eq!(
                uart.read_u32(status).unwrap() & (1 << empty_bit),
                0,
                "{layout:?}: RX-empty must clear when data is present"
            );
        }
    }

    #[test]
    fn test_uart_lpuart_tie_and_tcie_raise_irq() {
        // CTRL (0x08): TIE = bit 23, TCIE = bit 22. Either pends the LPUART IRQ.
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Lpuart);
        uart.write_u32(0x08, 1 << 23).unwrap(); // TIE
        assert!(uart.tick().irq, "TIE must pend");

        let mut uart = Uart::new_with_layout(UartRegisterLayout::Lpuart);
        uart.write_u32(0x08, 1 << 22).unwrap(); // TCIE
        assert!(uart.tick().irq, "TCIE must pend");

        // TE alone (bit 19, transmitter enable — not an interrupt) must not pend.
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Lpuart);
        uart.write_u32(0x08, 1 << 19).unwrap();
        assert!(!uart.tick().irq, "TE alone must not pend");
    }

    #[test]
    fn test_uart_lpuart_rx_sets_rdrf_and_reads_data() {
        let uart = Uart::new_with_layout(UartRegisterLayout::Lpuart);
        uart.rx_buffer().lock().unwrap().push_back(b'Z');
        // RDRF (STAT bit 21) sits at byte 2 bit 5 → read 0x06 has bit 5 set.
        assert_eq!(uart.read(0x06).unwrap(), 0xC0 | (1 << 5));
        // DATA read at 0x0C pops the byte.
        assert_eq!(uart.read(0x0C).unwrap(), b'Z');
        assert_eq!(uart.read(0x06).unwrap(), 0xC0, "RDRF clears once drained");
    }

    #[test]
    fn test_uart_tick_raises_irq_for_tcie() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);

        // CR1 bit 6 = TCIE for STM32F1.
        uart.write(0x0C, 1 << 6).unwrap();

        assert!(uart.tick().irq);
    }

    // ── Phase 2B.3b: event-scheduler path ────────────────────────────────
    // These exercise the same `advance_one_tick` core as `tick()` but via the
    // scheduler hooks. The hooks aren't feature-gated (only the Machine/bus
    // *callers* are), so they run in both build configs.

    #[test]
    fn event_path_arms_and_raises_own_irq_for_tcie() {
        use crate::bus::SystemBus;
        use crate::sched::EventScheduler;

        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        uart.write(0x0C, 1 << 6).unwrap(); // TCIE → active work

        // Arms exactly one WAKE; a second take is a no-op (already scheduled).
        assert_eq!(
            uart.take_scheduled_events(),
            vec![(0, super::UART_WAKE_TOKEN)]
        );
        assert!(uart.take_scheduled_events().is_empty());

        // on_event raises the UART's *own* IRQ and re-arms while TCIE is set.
        let mut sched = EventScheduler::new();
        let mut bus = SystemBus::empty();
        let r = uart.on_event(super::UART_WAKE_TOKEN, &mut sched, &mut bus);
        assert!(r.raise_own_irq, "event path must request the own-IRQ pend");
        assert_eq!(r.reschedule_delay, Some(1), "re-arm while interrupt is set");
    }

    #[test]
    fn event_path_stops_when_interrupt_cleared() {
        use crate::bus::SystemBus;
        use crate::sched::EventScheduler;

        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        uart.write(0x0C, 1 << 6).unwrap(); // TCIE on
        let _ = uart.take_scheduled_events();

        // Clear TCIE → next event raises no IRQ and does not re-arm.
        uart.write(0x0C, 0).unwrap();
        let mut sched = EventScheduler::new();
        let mut bus = SystemBus::empty();
        let r = uart.on_event(super::UART_WAKE_TOKEN, &mut sched, &mut bus);
        assert!(!r.raise_own_irq);
        assert_eq!(r.reschedule_delay, None, "idle UART stops scheduling");
        // And it won't re-arm itself with no active work.
        assert!(uart.take_scheduled_events().is_empty());
    }

    #[test]
    fn event_path_paces_attached_stream_rx() {
        use super::UartStreamDevice;
        use crate::bus::SystemBus;
        use crate::sched::EventScheduler;

        struct OneByte(u8);
        impl UartStreamDevice for OneByte {
            fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
                Some(self.0)
            }
        }

        let mut uart = Uart::new();
        uart.attach_stream(Box::new(OneByte(b'G')));

        // A stream attached at setup is "active work" → arms at bootstrap.
        assert_eq!(
            uart.take_scheduled_events(),
            vec![(0, super::UART_WAKE_TOKEN)]
        );

        let mut sched = EventScheduler::new();
        let mut bus = SystemBus::empty();
        let rx = uart.rx_buffer();
        uart.on_event(super::UART_WAKE_TOKEN, &mut sched, &mut bus);
        assert_eq!(rx.lock().unwrap().front().copied(), Some(b'G'));
    }

    #[test]
    fn attached_stream_observes_firmware_tx_bytes() {
        use super::UartStreamDevice;
        use std::sync::{Arc, Mutex};

        struct Recorder(Arc<Mutex<Vec<u8>>>);
        impl UartStreamDevice for Recorder {
            fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
                None
            }
            fn on_tx_byte(&mut self, byte: u8) {
                self.0.lock().unwrap().push(byte);
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut uart = Uart::new(); // Stm32F1 layout
        uart.set_sink(None, false); // disable stdout echo
        uart.attach_stream(Box::new(Recorder(seen.clone())));

        // Stm32F1: writing the DR alias at offset 0x00 transmits a byte.
        uart.write(0x00, 0x42).unwrap();

        assert_eq!(*seen.lock().unwrap(), vec![0x42]);
    }

    #[test]
    fn uart_trace_snapshot_records_tx_and_rx_without_draining_buffers() {
        use super::UartStreamDevice;

        struct OneByte(Option<u8>);
        impl UartStreamDevice for OneByte {
            fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
                self.0.take()
            }
        }

        let mut uart = Uart::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);
        uart.attach_stream(Box::new(OneByte(Some(0x33))));

        uart.write(0x04, 0x42).unwrap();
        uart.tick();

        let trace = uart.trace_snapshot();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].direction, "tx");
        assert_eq!(trace[0].byte, 0x42);
        assert_eq!(trace[1].direction, "rx");
        assert_eq!(trace[1].byte, 0x33);
        assert_eq!(sink.lock().unwrap().as_slice(), &[0x42]);
        assert_eq!(uart.read(0x04).unwrap(), 0x33);
    }
}

#[cfg(test)]
mod v2_irq_gating_tests {
    use super::*;
    use crate::Peripheral;

    /// CR1.TE (bit 3) must not raise the UART interrupt on the v2 layout —
    /// only TXEIE (bit 7) / TCIE (bit 6) do. Silicon-pinned on the bench
    /// NUCLEO-H563ZI (2026-06-11).
    #[test]
    fn v2_te_does_not_raise_irq_txeie_does() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
        // FIFOEN | TE | RE | UE — the embassy blocking-mode CR1.
        uart.write_u32(0x00, 0x2000_000D).unwrap();
        assert!(!uart.tick().irq, "TE alone must not pend");
        uart.write_u32(0x00, 0x2000_008D).unwrap(); // + TXEIE
        assert!(uart.tick().irq, "TXEIE must pend");
        uart.write_u32(0x00, 0x2000_004D).unwrap(); // TCIE instead
        assert!(uart.tick().irq, "TCIE must pend");
    }
}

/// Wakeup-observability gates: when may a `Uart` hold a per-cycle scheduler
/// wakeup open just to re-assert a level-triggered own-IRQ?
#[cfg(test)]
mod wakeup_observability_tests {
    use super::*;
    use crate::Peripheral;

    // ── IRQ-observability gate for the per-cycle wakeup ──────────────────
    // A `Uart` holding TXEIE/TCIE re-arms a scheduler event EVERY guest cycle
    // (`reschedule_delay: Some(1)`), which pins the whole machine's batch width
    // to 1. That is the right thing to do when someone can observe the level:
    // the machine pends the entry's `irq`. It is pure waste when the entry has
    // NO irq, because `apply_event_result` drops `raise_own_irq` on the floor —
    // and on that path `advance_one_tick` mutates nothing either. The ESP32-C3
    // is exactly that bus (chip yaml declares `uart0` with no `irq:`), where
    // this cost ~97% of all batch clamps and held the shipped display-workshop
    // lab at RTF 0.003.
    //
    // These pin the decision table so the collapse cannot come back.

    #[test]
    fn irqless_uart_holding_txeie_schedules_no_wakeups() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        uart.attach_irq_line(None); // the C3 `uart0` case
        uart.write(0x0C, 1 << 7).unwrap(); // TXEIE

        assert!(
            uart.take_scheduled_events().is_empty(),
            "an IRQ-less UART must not arm a per-cycle wakeup for a level nobody \
             can observe (the machine drops `raise_own_irq` with no entry irq)"
        );
        assert!(
            !uart.has_active_work(),
            "TXEIE alone is not 'active work' when the own-IRQ is unobservable"
        );
    }

    #[test]
    fn irq_wired_uart_holding_txeie_still_wakes_every_cycle() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        uart.attach_irq_line(Some(37)); // a real NVIC line (e.g. an STM32 lab)
        uart.write(0x0C, 1 << 7).unwrap(); // TXEIE

        assert_eq!(
            uart.take_scheduled_events(),
            vec![(0, UART_WAKE_TOKEN)],
            "a wired level-triggered IRQ must keep its exact legacy cadence"
        );
    }

    #[test]
    fn uart_defaults_to_wiring_its_irq_when_never_told() {
        // Buses that bypass the attach choke points (hand-built `PeripheralEntry`
        // literals) never call `attach_irq_line`. The conservative default must
        // preserve the legacy cadence — same contract as `attach_cycle_clock`.
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        uart.write(0x0C, 1 << 7).unwrap(); // TXEIE, no attach_irq_line call
        assert!(
            uart.has_active_work(),
            "without an explicit attach, a UART must assume its IRQ is wired"
        );
    }

    #[test]
    fn irqless_uart_still_wakes_for_real_work() {
        // The gate narrows ONLY the unobservable-level arm. Anything that
        // actually mutates state or emits bytes must still be scheduled.
        struct Silent;
        impl UartStreamDevice for Silent {
            fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
                None
            }
            fn on_tx_byte(&mut self, _byte: u8) {}
        }

        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        uart.attach_irq_line(None);
        uart.attach_stream(Box::new(Silent));
        assert!(
            uart.has_active_work(),
            "an attached RX stream is real work (byte pacing) regardless of IRQ wiring"
        );
        assert_eq!(
            uart.take_scheduled_events(),
            vec![(0, UART_WAKE_TOKEN)],
            "stream pacing must still arm on an IRQ-less bus"
        );
    }
}
