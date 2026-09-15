// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `Session::inject_can` on both CAN controller models: an STM32F103 bxCAN
//! (acceptance-filtered RX FIFO0) and an STM32H563 FDCAN (M_CAN RX FIFO0 in
//! message RAM).
//!
//! No CAN firmware is involved. Each test loads the chip's committed Tier-1
//! image, never runs its CAN code, and plays firmware itself through
//! `write_u32`: enable the controller clock, leave initialization, set a filter.
//! What the frame did is then read back from the registers firmware would
//! poll, at their RM0008 / RM0481 addresses.

mod common;

use labwired_core::bus::bus_trace::{BusDir, BusPayload};
use labwired_core::session::{
    AddrOrSymbol, CanFrame, CanRxRejection, OpenOptions, Session, SessionError,
};
use labwired_core::system::builder::*;

// STM32F103 (RM0008): RCC_APB1ENR.CAN1EN, bxCAN1 registers.
const F103_RCC_APB1ENR: u64 = 0x4002_101C;
const CAN1EN: u32 = 1 << 25;
const BXCAN: u64 = 0x4000_6400;
const BXCAN_RF0R: u64 = BXCAN + 0x00C;
const BXCAN_FS1R: u64 = BXCAN + 0x20C;
const BXCAN_FA1R: u64 = BXCAN + 0x21C;
const BXCAN_RI0R: u64 = BXCAN + 0x1B0;
const BXCAN_RDT0R: u64 = BXCAN + 0x1B4;
const BXCAN_RDL0R: u64 = BXCAN + 0x1B8;
const BXCAN_RDH0R: u64 = BXCAN + 0x1BC;

// STM32H563 (RM0481): RCC_APB1HENR.FDCAN1EN, FDCAN1 registers, SRAMCAN.
const H563_RCC_APB1HENR: u64 = 0x4402_0CA0;
const FDCAN1EN: u32 = 1 << 9;
const FDCAN: u64 = 0x4000_A400;
const FDCAN_CCCR: u64 = FDCAN + 0x018;
const FDCAN_IR: u64 = FDCAN + 0x050;
const FDCAN_IE: u64 = FDCAN + 0x054;
const FDCAN_ILE: u64 = FDCAN + 0x05C;
const FDCAN_RXF0S: u64 = FDCAN + 0x090;
/// First RX FIFO0 element: SRAMCAN (FDCAN + 0x800) + 0xB0.
const FDCAN_RF0_ELEMENT: u64 = FDCAN + 0x800 + 0xB0;
const IR_RF0N: u32 = 1 << 0;
const IR_RF0L: u32 = 1 << 2;
/// NVIC_ISPR1 (IRQ 32..63); FDCAN1_IT0 is IRQ 39.
const NVIC_ISPR1: u64 = 0xE000_E204;
const FDCAN1_IT0_BIT: u32 = 1 << (39 - 32);

fn open(f: &common::Fixture) -> Session {
    let blobs = BlobMap::new();
    Session::open(
        BuildRequest {
            chip: &f.chip,
            system: &f.manifest,
            firmware: FirmwareSource::Elf(&f.fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .unwrap()
}

fn open_f103() -> Session {
    open(&common::bare_chip_fixture(
        "stm32f103",
        "tests/fixtures/tier1/stm32f103.elf",
    ))
}

fn open_h563() -> Session {
    open(&common::bare_chip_fixture(
        "stm32h563",
        "tests/fixtures/tier1/stm32h563.elf",
    ))
}

fn reg(s: &Session, addr: u64) -> u32 {
    s.read_u32(AddrOrSymbol::Addr(addr)).unwrap()
}

fn set_bits(s: &mut Session, addr: u64, bits: u32) {
    let v = reg(s, addr);
    s.write_u32(AddrOrSymbol::Addr(addr), v | bits).unwrap();
}

fn rejected(r: Result<(), SessionError>) -> CanRxRejection {
    match r {
        Err(SessionError::CanRejected { reason, .. }) => reason,
        other => panic!("expected CanRejected, got {other:?}"),
    }
}

/// The CAN `rx` events in a batch of frames, as (bus, id, extended, data).
fn can_rx(frames: &[labwired_core::session::Frame]) -> Vec<(String, u32, bool, Vec<u8>)> {
    frames
        .iter()
        .filter_map(|f| match &f.payload {
            BusPayload::Can {
                direction: BusDir::Rx,
                id,
                data,
                extended,
                ..
            } => Some((f.bus.clone(), *id, *extended, data.clone())),
            _ => None,
        })
        .collect()
}

/// Clock on, and one accept-everything 32-bit mask filter on bank 0 (F0R1 =
/// F0R2 = 0 at reset).
fn bxcan_ready(s: &mut Session) {
    set_bits(s, F103_RCC_APB1ENR, CAN1EN);
    s.write_u32(AddrOrSymbol::Addr(BXCAN_FS1R), 1).unwrap();
    s.write_u32(AddrOrSymbol::Addr(BXCAN_FA1R), 1).unwrap();
}

#[test]
fn bxcan_receives_an_injected_frame_through_its_acceptance_filter() {
    let mut s = open_f103();
    let frame = CanFrame::classic(0x123, vec![0x01, 0x02, 0x03]);

    // The controller decides, in silicon order: clock, then filter.
    assert_eq!(
        rejected(s.inject_can("bxcan1", frame.clone())),
        CanRxRejection::Unclocked
    );
    set_bits(&mut s, F103_RCC_APB1ENR, CAN1EN);
    assert_eq!(
        rejected(s.inject_can("bxcan1", frame.clone())),
        CanRxRejection::NoFilterMatch,
        "a bxCAN with no active filter receives nothing"
    );
    assert_eq!(reg(&s, BXCAN_RF0R) & 0b11, 0);
    assert!(
        can_rx(&s.frames()).is_empty(),
        "a refused frame never reached the wire trace"
    );

    s.write_u32(AddrOrSymbol::Addr(BXCAN_FS1R), 1).unwrap();
    s.write_u32(AddrOrSymbol::Addr(BXCAN_FA1R), 1).unwrap();
    s.inject_can("bxcan1", frame).unwrap();

    assert_eq!(
        reg(&s, BXCAN_RF0R) & 0b11,
        1,
        "RF0R.FMP0: one message pending"
    );
    assert_eq!(reg(&s, BXCAN_RI0R), 0x123 << 21, "RI0R: STID, IDE = 0");
    assert_eq!(reg(&s, BXCAN_RDT0R) & 0xF, 3, "RDT0R.DLC");
    assert_eq!(reg(&s, BXCAN_RDL0R), 0x0003_0201);
    assert_eq!(
        can_rx(&s.frames()),
        vec![("bxcan1".to_string(), 0x123, false, vec![1, 2, 3])]
    );

    // An extended frame queues behind it; the third fills the FIFO.
    let ext = CanFrame {
        id: 0x18DA_F110,
        data: vec![0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7],
        extended: true,
        fd: false,
        bitrate_switch: false,
        remote: false,
    };
    s.inject_can("bxcan1", ext.clone()).unwrap();
    s.inject_can("bxcan1", CanFrame::classic(0x7FF, vec![]))
        .unwrap();
    let rf0r = reg(&s, BXCAN_RF0R);
    assert_eq!(rf0r & 0b11, 3, "FMP0 = 3");
    assert_ne!(rf0r & (1 << 3), 0, "FULL0");
    assert_eq!(
        rejected(s.inject_can("bxcan1", CanFrame::classic(0x100, vec![9]))),
        CanRxRejection::FifoFull
    );
    assert_eq!(
        can_rx(&s.frames()),
        vec![
            ("bxcan1".to_string(), 0x18DA_F110, true, ext.data.clone()),
            ("bxcan1".to_string(), 0x7FF, false, vec![]),
        ],
        "the frame lost to a full FIFO is not traced as received"
    );

    // Release the head: the extended frame is next, with its 29-bit id.
    s.write_u32(AddrOrSymbol::Addr(BXCAN_RF0R), 1 << 5).unwrap();
    assert_eq!(reg(&s, BXCAN_RF0R) & 0b11, 2);
    assert_eq!(
        reg(&s, BXCAN_RI0R),
        (0x18DA_F110 << 3) | (1 << 2),
        "RI0R: EXID, IDE = 1"
    );
    assert_eq!(reg(&s, BXCAN_RDH0R), 0xA7A6_A5A4);
}

#[test]
fn bxcan_refuses_can_fd() {
    let mut s = open_f103();
    bxcan_ready(&mut s);
    let fd = CanFrame {
        id: 0x123,
        data: vec![0; 12],
        extended: false,
        fd: true,
        bitrate_switch: true,
        remote: false,
    };
    assert_eq!(
        rejected(s.inject_can("bxcan1", fd)),
        CanRxRejection::FdOnClassicController
    );
    assert_eq!(reg(&s, BXCAN_RF0R) & 0b11, 0);
}

#[test]
fn fdcan_receives_an_injected_frame_and_raises_its_rx_interrupt() {
    let mut s = open_h563();
    let payload: Vec<u8> = (0x10..0x1C).collect(); // 12 bytes: DLC 9
    let frame = CanFrame {
        id: 0x456,
        data: payload.clone(),
        extended: false,
        fd: true,
        bitrate_switch: true,
        remote: false,
    };

    assert_eq!(
        rejected(s.inject_can("fdcan1", frame.clone())),
        CanRxRejection::Unclocked
    );
    set_bits(&mut s, H563_RCC_APB1HENR, FDCAN1EN);
    assert_eq!(
        rejected(s.inject_can("fdcan1", frame.clone())),
        CanRxRejection::NotRunning,
        "CCCR.INIT is set out of reset"
    );

    s.write_u32(AddrOrSymbol::Addr(FDCAN_CCCR), 0).unwrap();
    s.write_u32(AddrOrSymbol::Addr(FDCAN_IE), IR_RF0N).unwrap();
    s.write_u32(AddrOrSymbol::Addr(FDCAN_ILE), 1).unwrap();
    // Let the machine run first, so the frame arrives into a machine that is
    // already under way rather than one about to bootstrap its scheduler.
    s.run_cycles(16).unwrap();
    s.inject_can("fdcan1", frame).unwrap();

    assert_eq!(reg(&s, FDCAN_RXF0S), 0x0001_0001, "F0PI = 1, F0FL = 1");
    assert_ne!(reg(&s, FDCAN_IR) & IR_RF0N, 0, "IR.RF0N");
    let r0 = reg(&s, FDCAN_RF0_ELEMENT);
    let r1 = reg(&s, FDCAN_RF0_ELEMENT + 4);
    assert_eq!(r0 & 0x1FFF_FFFF, 0x456 << 18, "R0: standard id");
    assert_eq!((r1 >> 16) & 0xF, 9, "R1.DLC 9 = 12 bytes");
    assert_ne!(r1 & (1 << 21), 0, "R1.FDF");
    assert_ne!(r1 & (1 << 20), 0, "R1.BRS");
    assert_eq!(reg(&s, FDCAN_RF0_ELEMENT + 8), 0x1312_1110);
    assert_eq!(reg(&s, FDCAN_RF0_ELEMENT + 16), 0x1B1A_1918);
    assert_eq!(
        can_rx(&s.frames()),
        vec![("fdcan1".to_string(), 0x456, false, payload)]
    );

    // The arrival reaches the core: FDCAN1_IT0 pends once time moves.
    assert_eq!(reg(&s, NVIC_ISPR1) & FDCAN1_IT0_BIT, 0);
    s.run_cycles(16).unwrap();
    assert_ne!(
        reg(&s, NVIC_ISPR1) & FDCAN1_IT0_BIT,
        0,
        "FDCAN1_IT0 pending"
    );

    s.inject_can("fdcan1", CanFrame::classic(0x1, vec![1]))
        .unwrap();
    s.inject_can("fdcan1", CanFrame::classic(0x2, vec![2]))
        .unwrap();
    assert_eq!(
        rejected(s.inject_can("fdcan1", CanFrame::classic(0x3, vec![3]))),
        CanRxRejection::FifoFull
    );
    let rxf0s = reg(&s, FDCAN_RXF0S);
    assert_eq!(rxf0s & 0x7F, 3, "F0FL");
    assert_ne!(rxf0s & (1 << 25), 0, "RF0L: message lost");
    assert_ne!(reg(&s, FDCAN_IR) & IR_RF0L, 0, "IR.RF0L");
}

#[test]
fn inject_can_needs_a_can_controller_and_a_valid_frame() {
    let mut s = open_f103();
    bxcan_ready(&mut s);
    let ok = CanFrame::classic(0x123, vec![1]);
    assert!(matches!(
        s.inject_can("can9", ok.clone()),
        Err(SessionError::UnknownPeripheral(name)) if name == "can9"
    ));
    assert!(matches!(
        s.inject_can("uart1", ok),
        Err(SessionError::NotACanController(name)) if name == "uart1"
    ));

    let invalid = |frame: CanFrame| {
        let mut s = open_f103();
        bxcan_ready(&mut s);
        matches!(
            s.inject_can("bxcan1", frame),
            Err(SessionError::InvalidCanFrame(_))
        )
    };
    let base = CanFrame::classic(0x123, vec![]);
    assert!(
        invalid(CanFrame {
            id: 0x800,
            ..base.clone()
        }),
        "11-bit id above 0x7FF"
    );
    assert!(invalid(CanFrame {
        id: 0x2000_0000,
        extended: true,
        ..base.clone()
    }));
    assert!(
        invalid(CanFrame {
            data: vec![0; 9],
            ..base.clone()
        }),
        "classic over 8 bytes"
    );
    assert!(
        invalid(CanFrame {
            data: vec![0; 13],
            fd: true,
            ..base.clone()
        }),
        "no FD DLC for 13"
    );
    assert!(
        invalid(CanFrame {
            bitrate_switch: true,
            ..base.clone()
        }),
        "BRS without FD"
    );
    assert!(
        invalid(CanFrame {
            fd: true,
            remote: true,
            ..base.clone()
        }),
        "remote FD"
    );
    assert_eq!(reg(&s, BXCAN_RF0R) & 0b11, 0, "nothing was delivered");
}

/// The bxCAN RX FIFO is not in the controller's serialized state at all, so
/// only a restore that replays the injection brings the frame back.
#[test]
fn restore_replays_injected_frames() {
    let mut s = open_f103();
    bxcan_ready(&mut s);
    s.inject_can("bxcan1", CanFrame::classic(0x321, vec![0xAB]))
        .unwrap();
    let snap = s.snapshot();

    s.inject_can("bxcan1", CanFrame::classic(0x322, vec![]))
        .unwrap();
    s.inject_can("bxcan1", CanFrame::classic(0x323, vec![]))
        .unwrap();
    assert_eq!(reg(&s, BXCAN_RF0R) & 0b11, 3);

    s.restore(&snap).unwrap();
    assert_eq!(
        reg(&s, BXCAN_RF0R) & 0b11,
        1,
        "one frame pending, as at the snapshot"
    );
    assert_eq!(reg(&s, BXCAN_RI0R), 0x321 << 21);
    assert_eq!(reg(&s, BXCAN_RDL0R) & 0xFF, 0xAB);
    assert_eq!(
        can_rx(&s.frames()),
        vec![("bxcan1".to_string(), 0x321, false, vec![0xAB])]
    );
}
