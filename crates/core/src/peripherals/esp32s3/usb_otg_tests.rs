// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use super::*;

fn dev() -> Esp32s3UsbOtg {
    Esp32s3UsbOtg::new(USB_OTG_SOURCE)
}

fn rd(p: &Esp32s3UsbOtg, off: u64) -> u32 {
    p.read_u32(off).unwrap()
}

fn wr(p: &mut Esp32s3UsbOtg, off: u64, v: u32) {
    p.write_u32(off, v).unwrap();
}

#[test]
fn source_id_is_38() {
    assert_eq!(USB_OTG_SOURCE, 38);
    assert_eq!(dev().source_id, 38);
}

#[test]
fn grstctl_reset_and_flushes_self_clear_with_ahbidle() {
    let mut p = dev();
    wr(&mut p, GRSTCTL, GRSTCTL_CSRST);
    assert_eq!(rd(&p, GRSTCTL), GRSTCTL_AHBIDLE);
    wr(
        &mut p,
        GRSTCTL,
        GRSTCTL_TXFFLSH | (0x10 << GRSTCTL_TXFNUM_SHIFT),
    );
    wr(&mut p, GRSTCTL, GRSTCTL_RXFFLSH);
    assert_eq!(rd(&p, GRSTCTL), GRSTCTL_AHBIDLE);
}

#[test]
fn identity_registers_are_read_only_and_tinyusb_compatible() {
    let mut p = dev();
    wr(&mut p, GSNPSID, 0xDEAD_BEEF);
    wr(&mut p, GHWCFG2, 0);
    assert_eq!(rd(&p, GSNPSID) >> 16, 0x4F54, "TinyUSB asserts the OT id");
    assert_eq!(rd(&p, GHWCFG2), GHWCFG2_VALUE);
    // NUMDEVEPS = 6 -> 7 endpoints; FS PHY present, no HS PHY.
    assert_eq!((GHWCFG2_VALUE >> 10) & 0xF, 6);
    assert_eq!((GHWCFG2_VALUE >> 8) & 3, 1);
    assert_eq!((GHWCFG2_VALUE >> 6) & 3, 0);
}

#[test]
fn gintsts_w1c_leaves_derived_bits_alone() {
    let mut p = dev();
    p.gintsts = GINTSTS_USBRST | GINTSTS_ENUMDONE;
    p.write(GINTSTS + 1, 1 << (12 - 8)).unwrap();
    assert_eq!(rd(&p, GINTSTS) & GINTSTS_USBRST, 0);
    assert_ne!(rd(&p, GINTSTS) & GINTSTS_ENUMDONE, 0);
    // NPTXFE is derived (EP0 FIFO empty) and survives a write of all ones.
    wr(&mut p, GINTSTS, u32::MAX);
    assert_eq!(rd(&p, GINTSTS), GINTSTS_NPTXFE);
}

#[test]
fn irq_needs_global_and_local_enable() {
    let mut p = dev();
    p.gintsts = GINTSTS_ENUMDONE;
    assert!(p.tick_result().explicit_irqs.is_none());
    p.gintmsk = GINTSTS_ENUMDONE;
    assert!(p.tick_result().explicit_irqs.is_none(), "GAHBCFG gates it");
    p.gahbcfg = GAHBCFG_GINTMSK;
    assert_eq!(
        p.tick_result().explicit_irqs.as_deref(),
        Some(&[USB_OTG_SOURCE][..])
    );
    wr(&mut p, GINTSTS, GINTSTS_ENUMDONE);
    assert!(p.tick_result().explicit_irqs.is_none());
}

#[test]
fn host_waits_for_the_device_to_connect() {
    let mut p = dev();
    // Interrupts on but still soft-disconnected (DCTL reset value).
    wr(&mut p, GAHBCFG, GAHBCFG_GINTMSK);
    for _ in 0..10 {
        p.tick_elapsed(p.us(1_000));
    }
    assert_eq!(p.log.resets, 0);
    assert_eq!(rd(&p, GINTSTS) & GINTSTS_USBRST, 0);
}

#[test]
fn host_polls_a_device_it_waits_on_instead_of_sleeping_to_the_timeout() {
    let mut p = dev();
    wr(&mut p, GAHBCFG, GAHBCFG_GINTMSK);
    let dctl = rd(&p, DCTL) & !DCTL_SDIS;
    wr(&mut p, DCTL, dctl);
    // Debounce, bus reset, then the reset the firmware has not serviced yet.
    let step = p.us(ATTACH_DEBOUNCE_US + BUS_RESET_US);
    for _ in 0..3 {
        p.tick_elapsed(step);
    }
    assert!(matches!(p.phase, Phase::AwaitReset { .. }), "{:?}", p.phase);
    let now = p.now();
    assert_eq!(p.host_step(), Some(now + p.us(HOST_POLL_US)));
    // The firmware clears USBRST/ENUMDNE: the next look moves on.
    wr(&mut p, GINTSTS, GINTSTS_USBRST | GINTSTS_ENUMDONE);
    let poll = p.us(HOST_POLL_US);
    p.tick_elapsed(poll);
    assert!(matches!(p.phase, Phase::Idle { .. }), "{:?}", p.phase);
}

#[test]
fn in_endpoint_sends_one_packet_per_token_and_completes() {
    let mut p = dev();
    let n = 1;
    wr(
        &mut p,
        DIEP_BASE + EP_STRIDE,
        64 | (EPCTL_TYPE_BULK << 18) | (1 << 15),
    );
    // 70 bytes = 2 packets.
    wr(
        &mut p,
        DIEP_BASE + EP_STRIDE + 0x10,
        70 | (2 << TSIZ_PKTCNT_SHIFT),
    );
    let ctl = rd(&p, DIEP_BASE + EP_STRIDE);
    wr(
        &mut p,
        DIEP_BASE + EP_STRIDE,
        ctl | EPCTL_EPENA | EPCTL_CNAK,
    );
    assert!(
        p.take_in(n).is_none(),
        "NAK until the packet is in the FIFO"
    );
    let payload: Vec<u8> = (0..70u8).collect();
    for c in payload.chunks(4) {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        wr(
            &mut p,
            FIFO_BASE + FIFO_PAGE * n as u64,
            u32::from_le_bytes(w),
        );
    }
    assert_eq!(rd(&p, DIEP_BASE + EP_STRIDE + 0x08) & DIEPINT_TXFE, 0);
    assert_eq!(p.take_in(n).unwrap(), payload[..64]);
    assert_eq!(p.take_in(n).unwrap(), payload[64..]);
    let int = rd(&p, DIEP_BASE + EP_STRIDE + 0x08);
    assert_ne!(int & EPINT_XFRC, 0);
    assert_ne!(int & DIEPINT_TXFE, 0, "FIFO drained, padding included");
    assert_eq!(rd(&p, DIEP_BASE + EP_STRIDE) & EPCTL_EPENA, 0);
    assert!(p.take_in(n).is_none());
}

#[test]
fn out_packet_appears_in_rx_fifo_with_status_and_counts_down_tsiz() {
    let mut p = dev();
    wr(&mut p, DOEP_BASE + 0x10, 7 | (1 << TSIZ_PKTCNT_SHIFT));
    wr(&mut p, DOEP_BASE, EPCTL_EPENA | EPCTL_CNAK);
    assert_eq!(p.give_out(0, &[1, 2, 3, 4, 5, 6, 7]), Some(7));
    assert_ne!(rd(&p, GINTSTS) & GINTSTS_RXFLVL, 0);
    // Already counted out of DOEPTSIZ when the entry is visible.
    assert_eq!(tsiz_xfrsiz(rd(&p, DOEP_BASE + 0x10)), 0);
    let st = rd(&p, GRXSTSP);
    assert_eq!((st >> 17) & 0xF, PKTSTS_RX_DATA);
    assert_eq!((st >> 4) & 0x7FF, 7);
    assert_eq!(rd(&p, FIFO_BASE), 0x0403_0201);
    assert_eq!(rd(&p, FIFO_BASE) & 0x00FF_FFFF, 0x0007_0605);
    assert_eq!((rd(&p, GRXSTSP) >> 17) & 0xF, PKTSTS_RX_COMPLETE);
    assert_eq!(rd(&p, GINTSTS) & GINTSTS_RXFLVL, 0);
}

#[test]
fn byte_wide_fifo_reads_pop_one_word() {
    let mut p = dev();
    wr(&mut p, DOEP_BASE + 0x10, 8 | (1 << TSIZ_PKTCNT_SHIFT));
    wr(&mut p, DOEP_BASE, EPCTL_EPENA | EPCTL_CNAK);
    p.give_out(0, &[0xA1, 0xA2, 0xA3, 0xA4, 0xB1, 0xB2, 0xB3, 0xB4]);
    let _ = rd(&p, GRXSTSP);
    let w0: Vec<u8> = (0..4).map(|i| p.read(FIFO_BASE + i).unwrap()).collect();
    let w1: Vec<u8> = (0..4).map(|i| p.read(FIFO_BASE + i).unwrap()).collect();
    assert_eq!(w0, [0xA1, 0xA2, 0xA3, 0xA4]);
    assert_eq!(w1, [0xB1, 0xB2, 0xB3, 0xB4]);
}

#[test]
fn snak_reports_nak_effective_and_epdis_disables() {
    let mut p = dev();
    wr(&mut p, DIEP_BASE, EPCTL_EPENA);
    let ctl = rd(&p, DIEP_BASE);
    wr(&mut p, DIEP_BASE, ctl | EPCTL_SNAK);
    assert_ne!(rd(&p, DIEP_BASE + 0x08) & DIEPINT_INEPNE, 0);
    assert_ne!(rd(&p, DIEP_BASE) & EPCTL_NAKSTS, 0);
    let ctl = rd(&p, DIEP_BASE);
    wr(&mut p, DIEP_BASE, ctl | EPCTL_EPDIS);
    assert_ne!(rd(&p, DIEP_BASE + 0x08) & EPINT_EPDISD, 0);
    assert_eq!(rd(&p, DIEP_BASE) & EPCTL_EPENA, 0);
}

// ── End to end: a slave-mode device driver against the scripted host ──

const DEVICE_DESC: [u8; 18] = [
    18, 1, 0x00, 0x02, 0xEF, 0x02, 0x01, 64, 0x41, 0x23, 0x70, 0x00, 0x00, 0x01, 1, 2, 0, 1,
];

/// Configuration: CDC-ACM comm interface 0 (+ notification EP 0x81),
/// data interface 1 (bulk OUT 0x02, bulk IN 0x82).
fn config_desc() -> Vec<u8> {
    let mut c = vec![9, 2, 0, 0, 2, 1, 0, 0x80, 50];
    c.extend([9, 4, 0, 0, 1, 0x02, 0x02, 0x00, 0]);
    c.extend([7, 5, 0x81, 0x03, 8, 0, 16]);
    c.extend([9, 4, 1, 0, 2, 0x0A, 0x00, 0x00, 0]);
    c.extend([7, 5, 0x02, 0x02, 64, 0, 0]);
    c.extend([7, 5, 0x82, 0x02, 64, 0, 0]);
    let len = c.len() as u16;
    c[2..4].copy_from_slice(&len.to_le_bytes());
    c
}

fn string_desc(s: &str) -> Vec<u8> {
    let mut d = vec![0, 3];
    for u in s.encode_utf16() {
        d.extend(u.to_le_bytes());
    }
    d[0] = d.len() as u8;
    d
}

/// The device side, in the order TinyUSB's dcd_dwc2 (slave mode) runs it.
#[derive(Default)]
struct Driver {
    ep0_in: Vec<u8>,
    ep0_in_sent: usize,
    pending_status_in: bool,
    setup_bytes: Vec<u8>,
    line_coding: Vec<u8>,
    dtr: bool,
    hello_sent: bool,
}

impl Driver {
    fn init(p: &mut Esp32s3UsbOtg) {
        wr(p, GRSTCTL, GRSTCTL_CSRST);
        wr(
            p,
            GINTMSK,
            GINTSTS_USBRST | GINTSTS_ENUMDONE | GINTSTS_RXFLVL | GINTSTS_IEPINT,
        );
        wr(p, GAHBCFG, GAHBCFG_GINTMSK);
        wr(p, DCTL, rd(p, DCTL) & !DCTL_SDIS);
    }

    fn ep0_send_next(&mut self, p: &mut Esp32s3UsbOtg) {
        let rest = &self.ep0_in[self.ep0_in_sent..];
        let n = rest.len().min(64);
        wr(p, DIEP_BASE + 0x10, n as u32 | (1 << TSIZ_PKTCNT_SHIFT));
        wr(p, DIEP_BASE, rd(p, DIEP_BASE) | EPCTL_EPENA | EPCTL_CNAK);
        for c in rest[..n].chunks(4) {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            wr(p, FIFO_BASE, u32::from_le_bytes(w));
        }
        self.ep0_in_sent += n;
    }

    fn ep0_in(&mut self, p: &mut Esp32s3UsbOtg, data: Vec<u8>) {
        self.ep0_in = data;
        self.ep0_in_sent = 0;
        self.ep0_send_next(p);
    }

    fn arm_ep0_out(p: &mut Esp32s3UsbOtg, len: u32) {
        wr(p, DOEP_BASE + 0x10, len | (1 << TSIZ_PKTCNT_SHIFT));
        wr(p, DOEP_BASE, rd(p, DOEP_BASE) | EPCTL_EPENA | EPCTL_CNAK);
    }

    fn setup(&mut self, p: &mut Esp32s3UsbOtg, s: [u8; 8]) {
        let w_value = u16::from_le_bytes([s[2], s[3]]);
        let w_length = u16::from_le_bytes([s[6], s[7]]) as usize;
        match (s[0], s[1]) {
            (0x80, 6) => {
                let mut d = match w_value >> 8 {
                    1 => DEVICE_DESC.to_vec(),
                    2 => config_desc(),
                    3 if w_value & 0xFF == 0 => vec![4, 3, 0x09, 0x04],
                    3 => string_desc("NanoESP32"),
                    _ => Vec::new(),
                };
                d.truncate(w_length);
                self.ep0_in(p, d);
            }
            (0x00, 9) => {
                // SET_CONFIGURATION: open bulk IN 0x82.
                wr(
                    p,
                    DIEP_BASE + 2 * EP_STRIDE,
                    64 | (EPCTL_TYPE_BULK << 18) | (1 << 15),
                );
                self.ep0_in(p, Vec::new());
            }
            (0x21, 0x20) => Self::arm_ep0_out(p, w_length as u32),
            (0x21, 0x22) => {
                self.dtr = w_value & 1 != 0;
                self.ep0_in(p, Vec::new());
            }
            _ => self.ep0_in(p, Vec::new()),
        }
    }

    /// One pass of the interrupt handler.
    fn service(&mut self, p: &mut Esp32s3UsbOtg) {
        let st = rd(p, GINTSTS) & rd(p, GINTMSK);
        if st & GINTSTS_USBRST != 0 {
            wr(p, GINTSTS, GINTSTS_USBRST);
            wr(p, GNPTXFSIZ, (16 << 16) | 240);
            wr(p, DIEPTXF1 + 4, (16 << 16) | 200);
            wr(p, DIEPMSK, EPINT_XFRC);
            wr(p, DAINTMSK, 0x0001_0007);
        }
        if st & GINTSTS_ENUMDONE != 0 {
            wr(p, GINTSTS, GINTSTS_ENUMDONE);
        }
        while rd(p, GINTSTS) & GINTSTS_RXFLVL != 0 {
            let s = rd(p, GRXSTSP);
            let bcnt = ((s >> 4) & 0x7FF) as usize;
            match (s >> 17) & 0xF {
                PKTSTS_SETUP_RX => {
                    let mut b = rd(p, FIFO_BASE).to_le_bytes().to_vec();
                    b.extend(rd(p, FIFO_BASE).to_le_bytes());
                    self.ep0_in.clear();
                    self.setup_bytes = b;
                }
                PKTSTS_SETUP_DONE => {
                    let s: [u8; 8] = self.setup_bytes[..8].try_into().unwrap();
                    self.setup(p, s);
                }
                PKTSTS_RX_DATA => {
                    let mut data = Vec::new();
                    for _ in 0..bcnt.div_ceil(4) {
                        data.extend(rd(p, FIFO_BASE).to_le_bytes());
                    }
                    data.truncate(bcnt);
                    self.line_coding = data;
                    if bcnt > 0 {
                        self.pending_status_in = true;
                    }
                }
                PKTSTS_RX_COMPLETE if self.pending_status_in => {
                    self.pending_status_in = false;
                    self.ep0_in(p, Vec::new());
                }
                _ => {}
            }
        }
        if rd(p, GINTSTS) & GINTSTS_IEPINT != 0 {
            let int = rd(p, DIEP_BASE + 0x08);
            wr(p, DIEP_BASE + 0x08, int);
            if int & EPINT_XFRC != 0 {
                if self.ep0_in_sent < self.ep0_in.len() {
                    self.ep0_send_next(p);
                } else if !self.ep0_in.is_empty() {
                    // Data stage done: arm the status-stage OUT ZLP.
                    self.ep0_in.clear();
                    Self::arm_ep0_out(p, 0);
                }
            }
            let int2 = rd(p, DIEP_BASE + 2 * EP_STRIDE + 0x08);
            wr(p, DIEP_BASE + 2 * EP_STRIDE + 0x08, int2);
        }
        if self.dtr && !self.hello_sent {
            self.hello_sent = true;
            let msg = b"LW_OK\r\n";
            let base = DIEP_BASE + 2 * EP_STRIDE;
            wr(p, base + 0x10, msg.len() as u32 | (1 << TSIZ_PKTCNT_SHIFT));
            wr(p, base, rd(p, base) | EPCTL_EPENA | EPCTL_CNAK);
            for c in msg.chunks(4) {
                let mut w = [0u8; 4];
                w[..c.len()].copy_from_slice(c);
                wr(p, FIFO_BASE + 2 * FIFO_PAGE, u32::from_le_bytes(w));
            }
        }
    }
}

#[test]
fn scripted_host_enumerates_opens_cdc_and_reads_bulk_in() {
    let mut p = dev();
    let sink = Arc::new(Mutex::new(Vec::new()));
    assert!(p.attach_usb_cdc_console(Some(sink.clone())));
    Driver::init(&mut p);
    let mut drv = Driver::default();
    // 50 ms of bus time in 10 µs steps.
    for _ in 0..5_000 {
        p.tick_elapsed(p.us(10));
        drv.service(&mut p);
        p.tick_elapsed(0);
    }
    let log = p.host_log().clone();
    let text = log.lines().join("\n");
    assert_eq!(log.resets, 1, "{text}");
    assert_eq!(log.address, Some(5), "{text}");
    assert_eq!(log.device_descriptor, DEVICE_DESC, "{text}");
    assert_eq!(log.config_descriptor, config_desc(), "{text}");
    assert!(log.strings.iter().any(|(_, s)| s == "NanoESP32"), "{text}");
    assert!(log.control.iter().all(|(_, _, ok)| *ok), "{text}");
    // The terminal opened CDC interface 0 at 115200 8N1 with DTR|RTS.
    assert_eq!(drv.line_coding, [0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08]);
    assert!(drv.dtr, "{text}");
    assert_eq!(sink.lock().unwrap().as_slice(), b"LW_OK\r\n", "{text}");
    assert_eq!(p.phase, Phase::Done);
}

#[test]
fn without_a_console_the_port_is_never_opened() {
    let mut p = dev();
    Driver::init(&mut p);
    let mut drv = Driver::default();
    for _ in 0..5_000 {
        p.tick_elapsed(p.us(10));
        drv.service(&mut p);
    }
    assert_eq!(p.host_log().address, Some(5));
    assert!(!drv.dtr, "no terminal: DTR stays low");
}
