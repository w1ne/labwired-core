//! Bounded functional RK3x v1, matching the unchanged Linux 6.18.56 driver.
//!
//! Aligned little-endian 32-bit MMIO only; eight-word TX/RX buffers. Transfers
//! complete synchronously at the count write, not at START. Fixed functional
//! clocks/tuning readback do not model CRU, DMA, pinmux, wire timing or stretching.
//! BTF/BRF are not generated. LASTACK is a master terminal NACK, never NAKRCV.
//! IGNORE_NAK (ACTACK=0) is bounded as stop-on-NACK too; absent targets never
//! produce successful completion. Mode3 and multi-lane MRXADDR are unsupported.
use super::gic::Gic;
use crate::{
    peripherals::device::{drain_i2c_pin_drives, I2cDevice},
    Peripheral, SimResult, SimulationError,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
const EN: u32 = 1;
const START: u32 = 8;
const STOP: u32 = 16;
const MBTF: u32 = 4;
const MBRF: u32 = 8;
const NAK: u32 = 0x40;
const ALL: u32 = 0x7f;
const LOG_CAPACITY: usize = 256;
#[derive(Debug, Clone, Copy)]
struct Selection {
    index: usize,
    address: u8,
    reading: bool,
    generation: Option<u64>,
}
pub struct Rk3xI2c {
    // CON..FCNT. FCNT is our last successfully completed chunk's byte count
    // (including the first TX address), not a claimed hardware reset value.
    regs: [u32; 9],
    tx: [u8; 32],
    rx: [u8; 32],
    slaves: Vec<Box<dyn I2cDevice>>,
    selected: Option<Selection>,
    failed: bool,
    irq: bool,
    gic: Arc<Mutex<Gic>>,
    intid: usize,
    events: VecDeque<String>,
}
impl std::fmt::Debug for Rk3xI2c {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rk3xI2c")
            .field("intid", &self.intid)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}
impl Rk3xI2c {
    pub fn new(gic: Arc<Mutex<Gic>>, intid: usize) -> Self {
        Self {
            regs: [0; 9],
            tx: [0; 32],
            rx: [0; 32],
            slaves: vec![],
            selected: None,
            failed: false,
            irq: false,
            gic,
            intid,
            events: VecDeque::new(),
        }
    }
    pub fn push_slave(&mut self, s: Box<dyn I2cDevice>) {
        self.slaves.push(s);
    }
    /// Reset only controller-owned state. Attached devices and their registers
    /// are retained; no fabricated off-chip STOP or sensor reset occurs.
    pub fn reset(&mut self) {
        self.regs = [0; 9];
        self.tx = [0; 32];
        self.rx = [0; 32];
        self.selected = None;
        self.failed = false;
        self.events.clear();
        self.sync_irq();
    }
    fn unsupported(message: impl Into<String>) -> SimulationError {
        SimulationError::NotImplemented(message.into())
    }
    fn log(&mut self, text: String) {
        if self.events.len() == LOG_CAPACITY {
            self.events.pop_front();
        }
        self.events.push_back(text);
    }
    fn sync_irq(&mut self) {
        let level = self.regs[6] & self.regs[7] & ALL != 0;
        if level != self.irq {
            self.log(format!("IRQ{} {}", self.intid, u8::from(level)));
        }
        self.irq = level;
        self.gic.lock().unwrap().set_level(self.intid, level);
    }
    fn pending(&mut self, bits: u32) {
        self.regs[7] |= bits;
        self.sync_irq();
    }
    fn nak(&mut self) {
        self.log("NAK".into());
        self.selected = None;
        self.failed = true;
        // No completion is added: Linux must see NAK alone for -ENXIO.
        self.pending(NAK);
    }
    fn live(&self) -> bool {
        self.selected.is_some_and(|s| {
            self.slaves[s.index].transaction_generation() == s.generation
                && self.slaves[s.index].claims_address(s.address)
        })
    }
    fn start_selected(&mut self) -> bool {
        if !self.live() {
            self.nak();
            return false;
        }
        if let Some(s) = self.selected {
            self.slaves[s.index].start();
        }
        true
    }
    fn select(&mut self, address: u8) -> bool {
        let Some(index) = self.slaves.iter().position(|s| s.claims_address(address)) else {
            self.nak();
            return false;
        };
        self.selected = Some(Selection {
            index,
            address,
            reading: false,
            generation: self.slaves[index].transaction_generation(),
        });
        if !self.live() {
            self.nak();
            return false;
        }
        self.slaves[index].select_address(address);
        self.start_selected()
    }
    fn write_selected(&mut self, byte: u8) -> bool {
        if !self.live() {
            self.nak();
            return false;
        }
        if let Some(s) = self.selected {
            self.slaves[s.index].write(byte);
        }
        true
    }
    fn transmit(&mut self, count: usize) {
        if self.failed || (self.selected.is_some() && !self.live()) {
            self.nak();
            return;
        }
        if count == 0 {
            self.regs[8] = 0;
            self.pending(MBTF);
            return;
        }
        let first = self.selected.is_none();
        if first && !self.select(self.tx[0] >> 1) {
            return;
        }
        for i in usize::from(first)..count {
            if !self.write_selected(self.tx[i]) {
                return;
            }
        }
        self.regs[8] = count as u32;
        self.log(format!("TX {count}"));
        self.pending(MBTF);
    }
    fn receive(&mut self, count: usize) {
        if self.failed || (self.selected.is_some() && !self.live()) {
            self.nak();
            return;
        }
        if count == 0 {
            self.regs[8] = 0;
            self.pending(MBRF);
            return;
        }
        if self.selected.is_none() {
            let wire = self.regs[2] as u8;
            if !self.select(wire >> 1) {
                return;
            }
            if wire & 1 == 0 {
                for lane in 0..3 {
                    if self.regs[3] & (1 << (24 + lane)) != 0 {
                        let byte = (self.regs[3] >> (lane * 8)) as u8;
                        if !self.write_selected(byte) {
                            return;
                        }
                    }
                }
                // A combined write/read changes phase on this same instance;
                // the selected address is never delivered as register data.
                if !self.start_selected() {
                    return;
                }
            }
            if let Some(s) = &mut self.selected {
                s.reading = true;
            }
        }
        let mut bytes = [0; 32];
        for byte in &mut bytes[..count] {
            if !self.live() {
                self.nak();
                return;
            }
            if let Some(s) = self.selected {
                *byte = self.slaves[s.index].read();
            }
        }
        self.rx = bytes;
        self.regs[8] = count as u32;
        self.log(format!("RX {count}"));
        self.pending(MBRF);
    }
    fn request_stop(&mut self) {
        // A cut may have invalidated the selected target between chunks. Never
        // call the stale inner instance, but STOP remains usable for cleanup.
        if self.live() {
            if let Some(s) = self.selected {
                self.slaves[s.index].stop();
            }
        }
        self.selected = None;
        self.failed = false;
        self.log("STOP".into());
        self.pending(0x20);
    }
    fn check(offset: u64, width: u32) -> SimResult<()> {
        if width != 4 || offset % 4 != 0 {
            return Err(Self::unsupported(format!(
                "RK3x offset {offset:#x}, width {width}: aligned 32-bit MMIO only"
            )));
        }
        if offset <= 0x20 || (0x100..=0x11c).contains(&offset) || (0x200..=0x21c).contains(&offset)
        {
            Ok(())
        } else {
            Err(Self::unsupported(format!(
                "RK3x unmapped offset {offset:#x}"
            )))
        }
    }
    fn word(&self, offset: u64) -> SimResult<u32> {
        Self::check(offset, 4)?;
        if offset <= 0x20 {
            return Ok(self.regs[(offset / 4) as usize]);
        }
        let (bytes, base) = if offset < 0x200 {
            (&self.tx, 0x100)
        } else {
            (&self.rx, 0x200)
        };
        let i = (offset - base) as usize;
        Ok(u32::from_le_bytes([
            bytes[i],
            bytes[i + 1],
            bytes[i + 2],
            bytes[i + 3],
        ]))
    }
    fn validate_write(&self, offset: u64, v: u32) -> SimResult<()> {
        Self::check(offset, 4)?;
        match offset {
            0 if v & !0xff7f != 0 || (v >> 1) & 3 == 3 => {
                return Err(Self::unsupported("RK3x unsupported CON bits or mode3"))
            }
            8 if v & !0x010000ff != 0 => {
                return Err(Self::unsupported("RK3x MRXADDR supports valid lane0 only"))
            }
            12 if v & !0x07ffffff != 0 => {
                return Err(Self::unsupported(
                    "RK3x MRXRADDR supports three byte/valid lanes",
                ))
            }
            16 | 20 => {
                if v > 32 {
                    return Err(Self::unsupported("RK3x chunk count exceeds 32"));
                }
                let mode = (self.regs[0] >> 1) & 3;
                if self.regs[0] & EN == 0
                    || (offset == 16 && mode != 0)
                    || (offset == 20 && !matches!(mode, 1 | 2))
                {
                    return Err(Self::unsupported(
                        "RK3x chunk requires enabled matching transfer mode",
                    ));
                }
                if offset == 16 {
                    if self.selected.is_some_and(|s| s.reading)
                        || (v > 0 && self.selected.is_none() && !self.failed && self.tx[0] & 1 != 0)
                    {
                        return Err(Self::unsupported("RK3x TX requires a write address/phase"));
                    }
                } else if v > 0 && self.selected.is_none() && !self.failed {
                    if self.regs[2] & (1 << 24) == 0
                        || (self.regs[2] & 1 != 0 && self.regs[3] & 0x07000000 != 0)
                        || (mode == 2 && self.regs[2] & 1 == 0)
                    {
                        return Err(Self::unsupported(
                            "RK3x RX requires valid lane0 read/combined address",
                        ));
                    }
                } else if self.selected.is_some_and(|s| !s.reading) {
                    return Err(Self::unsupported(
                        "RK3x RX continuation requires read phase",
                    ));
                }
            }
            32 | 0x200..=0x21c => {
                return Err(Self::unsupported(
                    "RK3x read-only completed count/RX buffer",
                ))
            }
            _ => {}
        }
        Ok(())
    }
}
impl Peripheral for Rk3xI2c {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Self::check(offset, 1)?;
        Ok(0)
    }
    fn write(&mut self, offset: u64, _: u8) -> SimResult<()> {
        Self::check(offset, 1)
    }
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Self::check(offset, 2)?;
        Ok(0)
    }
    fn write_u16(&mut self, offset: u64, _: u16) -> SimResult<()> {
        Self::check(offset, 2)
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.word(offset)
    }
    fn write_u32(&mut self, offset: u64, v: u32) -> SimResult<()> {
        self.validate_write(offset, v)?;
        match offset {
            0 => {
                let old = self.regs[0];
                self.regs[0] = v;
                if v & EN == 0 {
                    self.selected = None;
                    self.failed = false;
                } else {
                    if v & START != 0 && old & START == 0 {
                        self.selected = None;
                        self.failed = false;
                        self.log("START".into());
                        self.pending(0x10);
                    }
                    if v & STOP != 0 && old & STOP == 0 {
                        self.request_stop();
                    }
                }
            }
            4 | 8 | 12 => self.regs[(offset / 4) as usize] = v,
            16 => {
                self.regs[4] = v;
                self.transmit(v as usize);
            }
            20 => {
                self.regs[5] = v;
                self.receive(v as usize);
            }
            24 => self.regs[6] = v & ALL,
            28 => self.regs[7] &= !(v & ALL),
            0x100..=0x11c => {
                let i = (offset - 0x100) as usize;
                self.tx[i..i + 4].copy_from_slice(&v.to_le_bytes());
            }
            _ => return Err(Self::unsupported("RK3x unsupported writable register")),
        }
        self.sync_irq();
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.word(offset & !3)
            .ok()
            .map(|v| ((v >> ((offset & 3) * 8)) & 255) as u8)
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.regs[6] & self.regs[7] & ALL != 0)
    }
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn drives_central_device_time(&self) -> bool {
        true
    }
    fn advance_attached_device_time_us(&mut self, us: u64) {
        for slave in &mut self.slaves {
            slave.advance_time_us(us);
        }
    }
    fn drain_attached_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        for slave in &mut self.slaves {
            drain_i2c_pin_drives(&mut **slave, out);
        }
    }
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for slave in &mut self.slaves {
            if slave.for_each_sim_input(f) {
                return true;
            }
        }
        false
    }
    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for slave in &self.slaves {
            crate::inspect::visit_i2c_device(&**slave, f);
        }
    }
    fn describe_registers(&self) -> Option<Vec<crate::inspect::RegisterSchema>> {
        let mut regs = vec![];
        for (i, name) in [
            "CON", "CLKDIV", "MRXADDR", "MRXRADDR", "MTXCNT", "MRXCNT", "IEN", "IPD", "FCNT",
        ]
        .into_iter()
        .enumerate()
        {
            regs.push(crate::inspect::RegisterSchema {
                name: name.into(),
                offset: i as u64 * 4,
                size: 32,
                access: if i == 8 { "ro" } else { "rw" },
                fields: vec![],
            });
        }
        for (prefix, base, access) in [("TX", 0x100, "rw"), ("RX", 0x200, "ro")] {
            for i in 0..8 {
                regs.push(crate::inspect::RegisterSchema {
                    name: format!("{prefix}{i}"),
                    offset: base + i * 4,
                    size: 32,
                    access,
                    fields: vec![],
                });
            }
        }
        Some(regs)
    }
    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut pi = crate::inspect::default_inspect(self, base, name, opts);
        pi.kind = "i2c".into();
        for slave in &self.slaves {
            pi.artifacts
                .extend(crate::inspect::i2c_artifacts(&**slave, opts));
        }
        pi
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        let mut logs = vec![crate::peripheral_log::PeripheralLog::new(
            "rk3x",
            self.events.iter().cloned().collect(),
        )];
        for slave in &self.slaves {
            logs.extend(slave.logs());
        }
        logs
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"registers":self.regs,"tx":self.tx,"rx":self.rx,"selected_address":self.selected.map(|s|s.address),"failed":self.failed,"irq":self.irq})
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
