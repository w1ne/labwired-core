// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Synchronous ADIv5 SWD. A successful read's data phase is the value this
//! transaction just produced. Posted reads are not implemented.

use crate::bus::SystemBus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwdAck {
    Ok,
    Wait,
    Fault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwdWdata {
    pub word: u32,
    pub parity: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwdHostError {
    MissingWriteData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwdTurn {
    NoAck,
    Ack {
        ack: SwdAck,
        data: Option<u32>,
        parity: Option<u8>,
    },
}

#[derive(Debug)]
pub struct SwdDp {
    idcode: u32,
    select: u32,
    req_dbg: bool,
    req_sys: bool,
    sticky_err: bool,
    wdata_err: bool,
    last_read: Option<u32>,
    last_ap_read: Option<u32>,
    csw: u32,
    tar: u32,
}

impl SwdDp {
    pub fn new(idcode: u32) -> Self {
        Self {
            idcode,
            select: 0,
            req_dbg: false,
            req_sys: false,
            sticky_err: false,
            wdata_err: false,
            last_read: None,
            last_ap_read: None,
            csw: 0x0000_0040,
            tar: 0,
        }
    }

    pub fn transact(
        &mut self,
        bus: &mut SystemBus,
        header: u8,
        wdata: Option<SwdWdata>,
    ) -> Result<SwdTurn, SwdHostError> {
        if !header_ok(header) {
            return Ok(SwdTurn::NoAck);
        }
        let ap = header & (1 << 1) != 0;
        let read = header & (1 << 2) != 0;
        let addr = (((header >> 3) & 1) << 2) | (((header >> 4) & 1) << 3);
        if !read && wdata.is_none() {
            return Err(SwdHostError::MissingWriteData);
        }
        if ap {
            self.ap_access(bus, read, addr, wdata)
        } else {
            self.dp_access(read, addr, wdata)
        }
    }

    fn dp_access(
        &mut self,
        read: bool,
        addr: u8,
        wdata: Option<SwdWdata>,
    ) -> Result<SwdTurn, SwdHostError> {
        let bank = self.select & 0xF;
        match (addr, read) {
            (0x0, true) => Ok(self.ok_read(self.idcode)),
            (0x0, false) => {
                let data = wdata.unwrap();
                if !data_parity_ok(data) {
                    self.wdata_err = true;
                    return Ok(ok_write());
                }
                if data.word & (1 << 2) != 0 {
                    self.sticky_err = false;
                }
                if data.word & (1 << 3) != 0 {
                    self.wdata_err = false;
                }
                Ok(ok_write())
            }
            (0x4, true) if bank == 0 => Ok(self.ok_read(self.ctrl_stat())),
            (0x4, false) if bank == 0 => {
                let data = wdata.unwrap();
                if !data_parity_ok(data) {
                    self.wdata_err = true;
                    return Ok(ok_write());
                }
                self.req_dbg = data.word & (1 << 28) != 0;
                self.req_sys = data.word & (1 << 30) != 0;
                Ok(ok_write())
            }
            (0x4, true) => Ok(self.ok_read(0)),
            (0x4, false) => {
                let data = wdata.unwrap();
                if !data_parity_ok(data) {
                    self.wdata_err = true;
                    return Ok(ok_write());
                }
                Ok(ok_write())
            }
            (0x8, true) => Ok(self.ok_read(self.last_read.unwrap_or(0))),
            (0x8, false) => {
                let data = wdata.unwrap();
                if !data_parity_ok(data) {
                    self.wdata_err = true;
                    return Ok(ok_write());
                }
                self.select = data.word;
                Ok(ok_write())
            }
            (0xC, true) => Ok(self.ok_read(self.last_ap_read.unwrap_or(0))),
            (0xC, false) => {
                let data = wdata.unwrap();
                if !data_parity_ok(data) {
                    self.wdata_err = true;
                    return Ok(ok_write());
                }
                Ok(ok_write())
            }
            _ => Ok(SwdTurn::NoAck),
        }
    }

    fn ap_access(
        &mut self,
        bus: &mut SystemBus,
        read: bool,
        addr: u8,
        wdata: Option<SwdWdata>,
    ) -> Result<SwdTurn, SwdHostError> {
        if !self.req_dbg || !self.req_sys {
            return Ok(SwdTurn::Ack {
                ack: SwdAck::Wait,
                data: None,
                parity: None,
            });
        }
        if self.sticky_err || self.wdata_err {
            return Ok(SwdTurn::Ack {
                ack: SwdAck::Fault,
                data: None,
                parity: None,
            });
        }
        let apsel = (self.select >> 24) & 0xFF;
        let bank = (self.select >> 4) & 0xF;
        if apsel != 0 {
            self.sticky_err = true;
            return Ok(SwdTurn::Ack {
                ack: SwdAck::Fault,
                data: None,
                parity: None,
            });
        }
        if !read {
            let data = wdata.unwrap();
            let good = (data.word.count_ones() & 1) as u8 == data.parity;
            if !good {
                self.wdata_err = true;
                return Ok(ok_write());
            }
            if bank != 0 {
                return Ok(ok_write());
            }
            match addr {
                0x0 => {
                    // Size is bits 0..2, AddrInc is bits 4..5. Bit 3 is not stored.
                    self.csw = (data.word & 0x37) | 0x40;
                }
                0x4 => self.tar = data.word,
                0xC => {
                    if let Err(()) = self.transfer(bus, false, data.word) {
                        return Ok(SwdTurn::Ack {
                            ack: SwdAck::Fault,
                            data: None,
                            parity: None,
                        });
                    }
                }
                _ => {}
            }
            return Ok(ok_write());
        }
        if bank != 0 {
            return Ok(self.ok_read(0));
        }
        let word = match addr {
            0x0 => self.csw,
            0x4 => self.tar,
            0xC => match self.transfer(bus, true, 0) {
                Ok(w) => w,
                Err(()) => {
                    return Ok(SwdTurn::Ack {
                        ack: SwdAck::Fault,
                        data: None,
                        parity: None,
                    });
                }
            },
            _ => 0,
        };
        self.last_ap_read = Some(word);
        Ok(self.ok_read(word))
    }

    fn transfer(&mut self, bus: &mut SystemBus, read: bool, wdata: u32) -> Result<u32, ()> {
        let size = self.csw & 0x7;
        let inc = (self.csw >> 4) & 0x3;
        if size != 2 || inc != 0 || (self.tar & 3) != 0 {
            self.sticky_err = true;
            return Err(());
        }
        use crate::Bus;
        if read {
            match bus.read_u32(self.tar as u64) {
                Ok(w) => Ok(w),
                Err(_) => {
                    self.sticky_err = true;
                    Err(())
                }
            }
        } else {
            match bus.write_u32(self.tar as u64, wdata) {
                Ok(()) => Ok(0),
                Err(_) => {
                    self.sticky_err = true;
                    Err(())
                }
            }
        }
    }

    fn ctrl_stat(&self) -> u32 {
        let mut w = 0u32;
        if self.req_sys {
            w |= (1 << 30) | (1 << 31);
        }
        if self.req_dbg {
            w |= (1 << 28) | (1 << 29);
        }
        if self.wdata_err {
            w |= 1 << 7;
        }
        if self.sticky_err {
            w |= 1 << 5;
        }
        w
    }

    fn ok_read(&mut self, word: u32) -> SwdTurn {
        self.last_read = Some(word);
        SwdTurn::Ack {
            ack: SwdAck::Ok,
            data: Some(word),
            parity: Some((word.count_ones() & 1) as u8),
        }
    }
}

fn data_parity_ok(data: SwdWdata) -> bool {
    (data.word.count_ones() & 1) as u8 == data.parity
}

fn ok_write() -> SwdTurn {
    SwdTurn::Ack {
        ack: SwdAck::Ok,
        data: None,
        parity: None,
    }
}

fn header_ok(header: u8) -> bool {
    let start = header & 1 == 1;
    let stop = (header >> 6) & 1 == 0;
    let park = (header >> 7) & 1 == 1;
    let body = (header >> 1) & 0xF;
    let parity = (header >> 5) & 1;
    start && stop && park && parity == (body.count_ones() & 1) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::system::cortex_m::{attach_swd_dp, configure_cortex_m};
    use crate::Machine;

    fn port() -> (SwdDp, Machine<crate::cpu::CortexM>) {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip =
            labwired_config::ChipDescriptor::from_file(root.join("configs/chips/nrf52840.yaml"))
                .expect("chip");
        let manifest: labwired_config::SystemManifest =
            serde_yaml::from_str("name: swd-gate\nchip: ignored\n").expect("manifest");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        let dp = attach_swd_dp(&mut machine.bus, &mut machine.cpu, 0x2BA0_1477);
        (dp, machine)
    }

    #[test]
    fn bad_header_does_not_stick() {
        let (mut dp, mut m) = port();
        let bad = 0xA5 ^ (1 << 5); // flipped parity
        assert!(matches!(
            dp.transact(&mut m.bus, bad, None).unwrap(),
            SwdTurn::NoAck
        ));
        match dp.transact(&mut m.bus, 0xA5, None).unwrap() {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(0x2BA0_1477),
                parity: Some(p),
            } => {
                assert_eq!(p, (0x2BA0_1477u32.count_ones() & 1) as u8);
            }
            other => panic!("expected IDCODE, got {other:?}"),
        }
    }

    #[test]
    fn bad_park_and_bad_stop_are_no_ack() {
        let (mut dp, mut m) = port();
        assert!(matches!(
            dp.transact(&mut m.bus, 0xA5 & !(1 << 7), None).unwrap(),
            SwdTurn::NoAck
        ));
        assert!(matches!(
            dp.transact(&mut m.bus, 0xA5 | (1 << 6), None).unwrap(),
            SwdTurn::NoAck
        ));
    }

    #[test]
    fn resend_repeats_idcode_and_rdbuff_stays_zero() {
        let (mut dp, mut m) = port();
        dp.transact(&mut m.bus, 0xA5, None).unwrap();
        // DP read of address 0x8: A[2]=0, A[3]=1, RnW=1, APnDP=0.
        let resend = swd_header(false, true, 0x8);
        match dp.transact(&mut m.bus, resend, None).unwrap() {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(word),
                ..
            } => {
                assert_eq!(word, 0x2BA0_1477);
            }
            other => panic!("RESEND {other:?}"),
        }
        let rdbuff = swd_header(false, true, 0xC);
        match dp.transact(&mut m.bus, rdbuff, None).unwrap() {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(0),
                ..
            } => {}
            other => panic!("RDBUFF {other:?}"),
        }
    }

    #[test]
    fn ignored_rdbuff_write_with_bad_parity_sets_wdata_err() {
        let (mut dp, mut m) = port();
        let word = 0x5000_0000u32;
        let turned = dp
            .transact(
                &mut m.bus,
                swd_header(false, false, 0xC),
                Some(SwdWdata {
                    word,
                    parity: ((word.count_ones() & 1) as u8) ^ 1,
                }),
            )
            .unwrap();
        assert!(matches!(
            turned,
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: None,
                ..
            }
        ));
        match dp
            .transact(&mut m.bus, swd_header(false, true, 0x4), None)
            .unwrap()
        {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(stat),
                ..
            } => {
                assert_eq!(stat & (1 << 7), 1 << 7, "WDATAERR, word {stat:#x}");
                assert_eq!(
                    stat & ((1 << 28) | (1 << 30)),
                    0,
                    "request bits must stay clear, word {stat:#x}"
                );
            }
            other => panic!("CTRL/STAT {other:?}"),
        }
    }

    pub(crate) fn swd_header(ap: bool, read: bool, addr: u8) -> u8 {
        let mut h = 1u8;
        if ap {
            h |= 1 << 1;
        }
        if read {
            h |= 1 << 2;
        }
        if addr & 0x4 != 0 {
            h |= 1 << 3;
        }
        if addr & 0x8 != 0 {
            h |= 1 << 4;
        }
        if ((h >> 1) & 0xF).count_ones() & 1 == 1 {
            h |= 1 << 5;
        }
        h |= 1 << 7;
        h
    }

    #[test]
    fn ap_waits_until_both_power_acks() {
        let (mut dp, mut m) = port();
        let ap_read = swd_header(true, true, 0x0);
        assert!(matches!(
            dp.transact(&mut m.bus, ap_read, None).unwrap(),
            SwdTurn::Ack {
                ack: SwdAck::Wait,
                data: None,
                ..
            }
        ));
        let ctrl_write = swd_header(false, false, 0x4);
        dp.transact(
            &mut m.bus,
            ctrl_write,
            Some(SwdWdata {
                word: 0x5000_0000,
                parity: 0,
            }),
        )
        .unwrap();
        let ctrl_read = swd_header(false, true, 0x4);
        match dp.transact(&mut m.bus, ctrl_read, None).unwrap() {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(word),
                ..
            } => {
                assert_eq!(word & 0xF000_0000, 0xF000_0000, "{word:#x}");
            }
            other => panic!("{other:?}"),
        }
    }

    fn powered(dp: &mut SwdDp, m: &mut Machine<crate::cpu::CortexM>) {
        let ctrl_write = swd_header(false, false, 0x4);
        let word = 0x5000_0000u32;
        dp.transact(
            &mut m.bus,
            ctrl_write,
            Some(SwdWdata {
                word,
                parity: (word.count_ones() & 1) as u8,
            }),
        )
        .unwrap();
    }

    #[test]
    fn drw_read_returns_the_word_this_transaction_loaded() {
        use crate::Bus;
        let (mut dp, mut m) = port();
        powered(&mut dp, &mut m);
        m.bus.write_u32(0x2000_0100, 0x4747_4553).unwrap();
        let csw = swd_header(true, false, 0x0);
        let csw_word = 0x0000_0042u32;
        dp.transact(
            &mut m.bus,
            csw,
            Some(SwdWdata {
                word: csw_word,
                parity: (csw_word.count_ones() & 1) as u8,
            }),
        )
        .unwrap();
        let tar = swd_header(true, false, 0x4);
        let tar_word = 0x2000_0100u32;
        dp.transact(
            &mut m.bus,
            tar,
            Some(SwdWdata {
                word: tar_word,
                parity: (tar_word.count_ones() & 1) as u8,
            }),
        )
        .unwrap();
        let drw = swd_header(true, true, 0xC);
        match dp.transact(&mut m.bus, drw, None).unwrap() {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(0x4747_4553),
                ..
            } => {}
            other => panic!("DRW {other:?}"),
        }
        let rdbuff = swd_header(false, true, 0xC);
        match dp.transact(&mut m.bus, rdbuff, None).unwrap() {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(0x4747_4553),
                ..
            } => {}
            other => panic!("first RDBUFF {other:?}"),
        }
        match dp.transact(&mut m.bus, rdbuff, None).unwrap() {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(0x4747_4553),
                ..
            } => {}
            other => panic!("second RDBUFF {other:?}"),
        }
    }

    #[test]
    fn size_zero_and_unmapped_and_bad_apsel_stick() {
        let (mut dp, mut m) = port();
        powered(&mut dp, &mut m);
        let drw = swd_header(true, true, 0xC);
        assert!(matches!(
            dp.transact(&mut m.bus, drw, None).unwrap(),
            SwdTurn::Ack {
                ack: SwdAck::Fault,
                ..
            }
        ));
        assert_eq!(ctrl(&mut dp, &mut m) & (1 << 5), 1 << 5);
        abort(&mut dp, &mut m, 1 << 2);
        assert_eq!(ctrl(&mut dp, &mut m) & (1 << 5), 0);

        let csw = swd_header(true, false, 0x0);
        let csw_word = 0x0000_0042u32;
        dp.transact(&mut m.bus, csw, Some(parity_word(csw_word)))
            .unwrap();
        let tar = swd_header(true, false, 0x4);
        // One past the 256KB SRAM. 0x4000_0000 is CLOCK on this chip, not a bus fault.
        dp.transact(&mut m.bus, tar, Some(parity_word(0x2004_0000)))
            .unwrap();
        let before = ctrl(&mut dp, &mut m);
        assert!(matches!(
            dp.transact(&mut m.bus, drw, None).unwrap(),
            SwdTurn::Ack {
                ack: SwdAck::Fault,
                ..
            }
        ));
        let rdbuff = swd_header(false, true, 0xC);
        match dp.transact(&mut m.bus, rdbuff, None).unwrap() {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(word),
                ..
            } => {
                assert_eq!(word, 0, "failed DRW must not update RDBUFF, got {word:#x}");
            }
            other => panic!("{other:?}"),
        }
        let _ = before;

        abort(&mut dp, &mut m, 1 << 2);
        let select = swd_header(false, false, 0x8);
        dp.transact(&mut m.bus, select, Some(parity_word(0x0100_0000)))
            .unwrap();
        assert!(matches!(
            dp.transact(&mut m.bus, drw, None).unwrap(),
            SwdTurn::Ack {
                ack: SwdAck::Fault,
                ..
            }
        ));
    }

    #[test]
    fn bad_write_parity_sets_wdata_err_and_keeps_the_word() {
        use crate::Bus;
        let (mut dp, mut m) = port();
        powered(&mut dp, &mut m);
        m.bus.write_u32(0x2000_0100, 0x1111_1111).unwrap();
        let csw = swd_header(true, false, 0x0);
        dp.transact(&mut m.bus, csw, Some(parity_word(0x0000_0042)))
            .unwrap();
        let tar = swd_header(true, false, 0x4);
        dp.transact(&mut m.bus, tar, Some(parity_word(0x2000_0100)))
            .unwrap();
        let drw_w = swd_header(true, false, 0xC);
        let turned = dp
            .transact(
                &mut m.bus,
                drw_w,
                Some(SwdWdata {
                    word: 0x2222_2222,
                    parity: ((0x2222_2222u32.count_ones() & 1) as u8) ^ 1,
                }),
            )
            .unwrap();
        assert!(matches!(
            turned,
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: None,
                ..
            }
        ));
        assert_eq!(m.bus.read_u32(0x2000_0100).unwrap(), 0x1111_1111);
        assert_eq!(ctrl(&mut dp, &mut m) & (1 << 7), 1 << 7);
        let drw_r = swd_header(true, true, 0xC);
        assert!(matches!(
            dp.transact(&mut m.bus, drw_r, None).unwrap(),
            SwdTurn::Ack {
                ack: SwdAck::Fault,
                ..
            }
        ));
        abort(&mut dp, &mut m, 1 << 3);
        assert_eq!(ctrl(&mut dp, &mut m) & (1 << 7), 0);
    }

    #[test]
    fn other_bank_write_checks_parity_before_ignore() {
        let (mut dp, mut m) = port();
        powered(&mut dp, &mut m);
        let select = swd_header(false, false, 0x8);
        dp.transact(&mut m.bus, select, Some(parity_word(0x10)))
            .unwrap();
        let csw = swd_header(true, false, 0x0);
        let word = 0x0000_0042u32;
        let turned = dp
            .transact(
                &mut m.bus,
                csw,
                Some(SwdWdata {
                    word,
                    parity: ((word.count_ones() & 1) as u8) ^ 1,
                }),
            )
            .unwrap();
        assert!(matches!(
            turned,
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: None,
                ..
            }
        ));
        assert_eq!(ctrl(&mut dp, &mut m) & (1 << 7), 1 << 7);
        dp.transact(&mut m.bus, select, Some(parity_word(0)))
            .unwrap();
        abort(&mut dp, &mut m, 1 << 3);
        match dp
            .transact(&mut m.bus, swd_header(true, true, 0x0), None)
            .unwrap()
        {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(csw_word),
                ..
            } => {
                assert_eq!(csw_word, 0x40, "dropped bank write, CSW {csw_word:#x}");
            }
            other => panic!("CSW {other:?}"),
        }
    }

    #[test]
    fn missing_write_data_does_not_touch_ctrl_stat() {
        let (mut dp, mut m) = port();
        let ctrl_write = swd_header(false, false, 0x4);
        let err = dp.transact(&mut m.bus, ctrl_write, None).unwrap_err();
        assert_eq!(err, SwdHostError::MissingWriteData);
        assert_eq!(ctrl(&mut dp, &mut m), 0);
    }

    fn parity_word(word: u32) -> SwdWdata {
        SwdWdata {
            word,
            parity: (word.count_ones() & 1) as u8,
        }
    }

    fn ctrl(dp: &mut SwdDp, m: &mut Machine<crate::cpu::CortexM>) -> u32 {
        let hdr = swd_header(false, true, 0x4);
        match dp.transact(&mut m.bus, hdr, None).unwrap() {
            SwdTurn::Ack { data: Some(w), .. } => w,
            other => panic!("{other:?}"),
        }
    }

    fn abort(dp: &mut SwdDp, m: &mut Machine<crate::cpu::CortexM>, bits: u32) {
        let hdr = swd_header(false, false, 0x0);
        dp.transact(&mut m.bus, hdr, Some(parity_word(bits)))
            .unwrap();
    }

    #[test]
    fn cpu_store_is_what_drw_returns() {
        use crate::Bus;
        use crate::Cpu;
        let (mut dp, mut m) = port();
        // str r0, [r1]; b .
        m.bus.write_u16(0x2000_0000, 0x6008).unwrap();
        m.bus.write_u16(0x2000_0002, 0xE7FE).unwrap();
        m.cpu.r0 = 0x4747_4553;
        m.cpu.r1 = 0x2000_0100;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        let mut saw = false;
        for _ in 0..8 {
            m.step().unwrap();
            if m.bus.read_u32(0x2000_0100).unwrap() == 0x4747_4553 {
                saw = true;
                break;
            }
        }
        assert!(saw, "CPU never stored the word");
        powered(&mut dp, &mut m);
        dp.transact(
            &mut m.bus,
            swd_header(true, false, 0x0),
            Some(parity_word(0x42)),
        )
        .unwrap();
        dp.transact(
            &mut m.bus,
            swd_header(true, false, 0x4),
            Some(parity_word(0x2000_0100)),
        )
        .unwrap();
        match dp
            .transact(&mut m.bus, swd_header(true, true, 0xC), None)
            .unwrap()
        {
            SwdTurn::Ack {
                ack: SwdAck::Ok,
                data: Some(0x4747_4553),
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn dhcsr_store_suppresses_the_next_store_inside_one_batch() {
        use crate::Bus;
        use crate::Cpu;
        let (_dp, mut m) = port();
        // str r0, [r1]; str r2, [r3]; b .
        m.bus.write_u16(0x2000_0000, 0x6008).unwrap();
        m.bus.write_u16(0x2000_0002, 0x601A).unwrap();
        m.bus.write_u16(0x2000_0004, 0xE7FE).unwrap();
        m.cpu.r0 = 0xA05F_0003;
        m.cpu.r1 = 0xE000_EDF0;
        m.cpu.r2 = 0xDEAD_BEEF;
        m.cpu.r3 = 0x2000_0100;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        let config = crate::SimulationConfig::default();
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 8)
            .unwrap();
        assert_eq!(m.bus.read_u32(0x2000_0100).unwrap(), 0, "sentinel retired");
        assert_eq!(m.cpu.pc, 0x2000_0002, "pc {:#x}", m.cpu.pc);
        assert_eq!(m.bus.read_u32(0xE000_EDF0).unwrap() & (1 << 17), 1 << 17);
        m.step().unwrap();
        assert_eq!(m.cpu.pc, 0x2000_0002);
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0001).unwrap();
        m.step().unwrap();
        assert_eq!(m.bus.read_u32(0x2000_0100).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn dhcsr_halt_blocks_the_thumb_fast_path_ram_store() {
        use crate::Bus;
        use crate::Cpu;
        let (_dp, mut m) = port();
        // str r0, [r1]; str r2, [r3]; b 0x2000_0000.
        // The backward branch is a real fast-block; `b .` is not.
        m.bus.write_u16(0x2000_0000, 0x6008).unwrap();
        m.bus.write_u16(0x2000_0002, 0x601A).unwrap();
        m.bus.write_u16(0x2000_0004, 0xE7FC).unwrap();
        m.cpu.r0 = 0;
        m.cpu.r1 = 0xE000_EDF0;
        m.cpu.r2 = 0xDEAD_BEEF;
        m.cpu.r3 = 0x2000_0100;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        // `step_batch` probes the Thumb fast path when the remaining budget
        // is at least 8. A tick of 1 never hands it that budget; 512 does.
        let config = crate::SimulationConfig {
            peripheral_tick_interval: crate::bus::RECOMMENDED_TICK_INTERVAL,
            ..crate::SimulationConfig::default()
        };
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 32)
            .unwrap();
        assert_eq!(
            m.bus.read_u32(0x2000_0100).unwrap(),
            0xDEAD_BEEF,
            "warmup must decode the RAM store"
        );
        assert!(
            m.cpu.decoded_entry(0x2000_0002).is_some(),
            "RAM store was not cached"
        );
        m.bus.write_u32(0x2000_0100, 0).unwrap();
        m.cpu.set_pc(0x2000_0002);
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0003).unwrap();
        assert_eq!(m.bus.read_u32(0xE000_EDF0).unwrap() & (1 << 17), 1 << 17);
        m.cpu
            .step_batch(
                &mut m.bus,
                &observers,
                &config,
                crate::bus::RECOMMENDED_TICK_INTERVAL,
            )
            .unwrap();
        assert_eq!(m.bus.read_u32(0x2000_0100).unwrap(), 0, "sentinel retired");
        assert_eq!(m.cpu.pc, 0x2000_0002, "pc {:#x}", m.cpu.pc);
    }

    #[test]
    fn jit_batch_also_suppresses_the_store_after_dhcsr() {
        use crate::Bus;
        use crate::Cpu;
        let (_dp, mut m) = port();
        m.bus.write_u16(0x2000_0000, 0x6008).unwrap();
        m.bus.write_u16(0x2000_0002, 0x601A).unwrap();
        m.bus.write_u16(0x2000_0004, 0xE7FE).unwrap();
        m.cpu.r0 = 0xA05F_0003;
        m.cpu.r1 = 0xE000_EDF0;
        m.cpu.r2 = 0xDEAD_BEEF;
        m.cpu.r3 = 0x2000_0100;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        let config = crate::SimulationConfig {
            cortex_m_jit_enabled: true,
            ..crate::SimulationConfig::default()
        };
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 8)
            .unwrap();
        assert_eq!(m.bus.read_u32(0x2000_0100).unwrap(), 0);
        assert_eq!(m.cpu.pc, 0x2000_0002, "pc {:#x}", m.cpu.pc);
    }

    #[test]
    fn jit_ready_block_does_not_run_when_already_halted() {
        use crate::Bus;
        use crate::Cpu;
        let (_dp, mut m) = port();
        // adds r0, #1; b 0x2000_0000.
        // Branch at 0x2000_0002: architectural PC+4 is 0x2000_0006, target
        // 0x2000_0000, delta -6, imm11 -3 → 0xE7FD. `b .` (0xE7FE) is not
        // this loop.
        m.bus.write_u16(0x2000_0000, 0x3001).unwrap();
        m.bus.write_u16(0x2000_0002, 0xE7FD).unwrap();
        m.cpu.r0 = 0;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        // Default profitable floor is 4. This block is two instructions;
        // left at the default it never installs and the interpreter's
        // halt check hides a missing JIT pre-check.
        let config = crate::SimulationConfig {
            cortex_m_jit_enabled: true,
            cortex_m_jit_min_block_instrs: 2,
            ..crate::SimulationConfig::default()
        };
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        // Hot threshold counts entries of one PC. Until the entry compiles,
        // the loop is two interpreted instructions, so 80 steps (40 entries)
        // stays cold. 200 crosses 50 and then runs the compiled block.
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 200)
            .unwrap();
        assert!(m.cpu.r0 > 0, "warmup r0 {}", m.cpu.r0);
        #[cfg(feature = "jit")]
        let runs = {
            let stats = m.cpu.jit_stats().expect("jit engine");
            assert!(
                stats.block_runs > 0,
                "warmup never reached Lookup::Ready: {stats:?}"
            );
            stats.block_runs
        };
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0003).unwrap();
        assert!(m.cpu.debug_halted());
        let r0 = m.cpu.r0;
        let pc = m.cpu.pc;
        let halted = m
            .cpu
            .step_batch(&mut m.bus, &observers, &config, 80)
            .unwrap();
        assert_eq!(
            m.cpu.r0, r0,
            "halted core retired the ready block (batch retired {halted})"
        );
        assert_eq!(m.cpu.pc, pc, "pc {:#x}", m.cpu.pc);
        #[cfg(feature = "jit")]
        {
            assert_eq!(halted, 0, "halted batch retired {halted}");
            let after = m.cpu.jit_stats().map(|s| s.block_runs).unwrap_or(0);
            assert_eq!(after, runs, "compiled block ran while halted");
        }
    }

    #[test]
    fn machine_reset_clears_debug_halt() {
        use crate::Bus;
        let (_dp, mut m) = port();
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0003).unwrap();
        assert!(m.cpu.debug_halted());
        m.reset().unwrap();
        assert!(!m.cpu.debug_halted());
        assert_eq!(m.bus.read_u32(0xE000_EDF0).unwrap() & (1 << 17), 0);
        // C_DEBUGEN stays across the reset. VC_CORERESET was clear, so the
        // core comes out running.
        assert_eq!(m.bus.read_u32(0xE000_EDF0).unwrap() & 1, 1);
    }

    #[test]
    fn c_step_retires_one_instruction_then_halts() {
        use crate::Bus;
        use crate::Cpu;
        let (_dp, mut m) = port();
        // adds r2, #1; adds r2, #1; b .
        m.bus.write_u16(0x2000_0000, 0x3201).unwrap();
        m.bus.write_u16(0x2000_0002, 0x3201).unwrap();
        m.bus.write_u16(0x2000_0004, 0xE7FE).unwrap();
        m.cpu.r2 = 0;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0003).unwrap();
        assert!(m.cpu.debug_halted());
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0005).unwrap();
        assert!(m.cpu.debug_step_pending());
        assert!(!m.cpu.debug_halted());
        let config = crate::SimulationConfig::default();
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        let n = m
            .cpu
            .step_batch(&mut m.bus, &observers, &config, 8)
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(m.cpu.r2, 1);
        assert_eq!(m.cpu.pc, 0x2000_0002);
        assert!(m.cpu.debug_halted());
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 8)
            .unwrap();
        assert_eq!(m.cpu.r2, 1);
        assert_eq!(m.cpu.pc, 0x2000_0002);
    }

    #[test]
    fn firmware_c_step_does_not_consume_the_arming_store() {
        use crate::Bus;
        use crate::Cpu;
        let (_dp, mut m) = port();
        // str r0, [r1]; adds r2, #1; adds r2, #1; b .
        m.bus.write_u16(0x2000_0000, 0x6008).unwrap();
        m.bus.write_u16(0x2000_0002, 0x3201).unwrap();
        m.bus.write_u16(0x2000_0004, 0x3201).unwrap();
        m.bus.write_u16(0x2000_0006, 0xE7FE).unwrap();
        m.cpu.r0 = 0xA05F_0005;
        m.cpu.r1 = 0xE000_EDF0;
        m.cpu.r2 = 0;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        let config = crate::SimulationConfig::default();
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 8)
            .unwrap();
        assert_eq!(
            m.cpu.r2, 0,
            "the arming store is not the stepped instruction"
        );
        assert_eq!(m.cpu.pc, 0x2000_0002);
        assert!(m.cpu.debug_step_pending());
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 8)
            .unwrap();
        assert_eq!(m.cpu.r2, 1);
        assert_eq!(m.cpu.pc, 0x2000_0004);
        assert!(m.cpu.debug_halted());
    }

    #[test]
    fn c_maskints_blocks_systick_and_lets_nmi_through() {
        use crate::Bus;
        use crate::Cpu;
        let (_dp, mut m) = port();
        // movs r0, #7
        m.bus.write_u16(0x2000_0000, 0x2007).unwrap();
        m.cpu.r0 = 0;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        m.cpu.set_vtor(0x2000_1000);
        // NMI handler: movs r3, #1
        m.bus.write_u32(0x2000_1008, 0x2000_2001).unwrap();
        m.bus.write_u16(0x2000_2000, 0x2301).unwrap();
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0009).unwrap();
        m.cpu.pending_exceptions[0] |= 1 << 15;
        let config = crate::SimulationConfig::default();
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 1)
            .unwrap();
        assert_eq!(m.cpu.r0, 7, "SysTick must not preempt under C_MASKINTS");
        assert_eq!(m.cpu.pc, 0x2000_0002);

        m.cpu.pending_exceptions[0] |= 1 << 2;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.r0 = 0;
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 1)
            .unwrap();
        assert_eq!(m.cpu.r0, 0, "NMI is taken before the movs");
        assert_eq!(m.cpu.pc & !1, 0x2000_2000);
    }

    #[test]
    fn vc_corereset_halts_on_reset() {
        use crate::Bus;
        let (_dp, mut m) = port();
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0001).unwrap();
        m.bus.write_u32(0xE000_EDFC, 1).unwrap();
        assert!(!m.cpu.debug_halted());
        m.reset().unwrap();
        assert!(m.cpu.debug_halted());
        assert_eq!(m.bus.read_u32(0xE000_EDF0).unwrap() & 1, 1);
    }

    #[test]
    fn vc_buserr_halts_instead_of_pending_the_fault() {
        use crate::Bus;
        use crate::Cpu;
        let (_dp, mut m) = port();
        // str r0, [r1] — past the 256 KB nRF52840 SRAM.
        m.bus.write_u16(0x2000_0000, 0x6008).unwrap();
        m.cpu.r0 = 0x11;
        m.cpu.r1 = 0x2010_0000;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0001).unwrap();
        m.bus.write_u32(0xE000_EDFC, 1 << 8).unwrap();
        let config = crate::SimulationConfig::default();
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 4)
            .unwrap();
        assert!(m.cpu.debug_halted());
        assert_eq!(m.cpu.pc, 0x2000_0000);
        assert_eq!(m.cpu.pending_exceptions[0] & ((1 << 5) | (1 << 3)), 0);
    }

    #[test]
    fn wfi_sets_dhcsr_s_sleep_until_the_next_instruction() {
        use crate::Bus;
        use crate::Cpu;
        let (_dp, mut m) = port();
        // wfi (0xBF30); movs r0, #1. 0xBF20 is WFE.
        m.bus.write_u16(0x2000_0000, 0xBF30).unwrap();
        m.bus.write_u16(0x2000_0002, 0x2001).unwrap();
        m.cpu.r0 = 0;
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_2000);
        m.bus.write_u32(0xE000_EDF0, 0xA05F_0001).unwrap();
        let config = crate::SimulationConfig::default();
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 1)
            .unwrap();
        assert_ne!(m.bus.read_u32(0xE000_EDF0).unwrap() & (1 << 18), 0);
        m.cpu
            .step_batch(&mut m.bus, &observers, &config, 1)
            .unwrap();
        assert_eq!(m.cpu.r0, 1);
        assert_eq!(m.bus.read_u32(0xE000_EDF0).unwrap() & (1 << 18), 0);
    }
}
