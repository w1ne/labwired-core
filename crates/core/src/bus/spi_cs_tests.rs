// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GPIO chip select reaches STM32 SPI devices (`bus/spi_cs.rs`).

use super::SystemBus;
use crate::peripherals::{
    gpio::{GpioPort, GpioRegisterLayout},
    spi::{Spi, SpiDevice, SpiRegisterLayout},
};
use crate::Bus;
use std::sync::{Arc, Mutex};

const GPIOA: u64 = 0x4001_0800;
const GPIOB: u64 = 0x4001_0C00;
const SPI1: u64 = 0x4001_3000;
// STM32F1 GPIO: CRL +0x00, CRH +0x04, BSRR +0x10.
const CRL: u64 = 0x00;
const CRH: u64 = 0x04;
const BSRR: u64 = 0x10;

type Log = Arc<Mutex<Vec<String>>>;

/// Records CS edges and every byte it is clocked, answering with its tag.
struct Probe {
    tag: &'static str,
    cs: &'static str,
    miso: u8,
    log: Log,
}

impl SpiDevice for Probe {
    fn transfer(&mut self, mosi: u8) -> u8 {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:{mosi:02x}", self.tag));
        self.miso
    }
    fn cs_pin(&self) -> &str {
        self.cs
    }
    fn cs_select(&mut self) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:select", self.tag));
    }
    fn cs_release(&mut self) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:release", self.tag));
    }
}

fn bus_with_two_slaves(log: &Log) -> SystemBus {
    let mut bus = SystemBus::new();
    // `new()` pre-populates an F1 `gpioa`; start from an empty set.
    bus.peripherals.clear();
    for (name, base) in [("gpioa", GPIOA), ("gpiob", GPIOB)] {
        bus.add_peripheral(
            name,
            base,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32F1)),
        );
    }
    bus.add_peripheral(
        "spi1",
        SPI1,
        0x400,
        None,
        Box::new(Spi::new_with_layout(SpiRegisterLayout::Stm32)),
    );
    for (tag, cs, miso) in [("a", "PA4", 0xA1), ("b", "PB12", 0xB2)] {
        bus.attach_spi_device(
            "spi1",
            Box::new(Probe {
                tag,
                cs,
                miso,
                log: log.clone(),
            }),
        )
        .unwrap();
    }
    bus
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

/// Clock one byte through SPI1 as a master and check what DR reads back.
///
/// The device is consulted at the DR write in both builds, so the log
/// assertions hold everywhere. The frame only completes into DR on the legacy
/// walk: under `event-scheduler` the SPI is timed by the `Machine`'s
/// scheduler, which a bare bus does not run, so the MISO check is legacy-only
/// (the STM32L151 onboarding smoke covers it end to end).
fn xfer(bus: &mut SystemBus, mosi: u8, miso: Option<u8>) {
    Bus::write_u32(bus, SPI1 + 0x0C, u32::from(mosi)).unwrap();
    if crate::cycle_clock::event_scheduler_enabled() {
        return;
    }
    for _ in 0..4096 {
        Bus::tick_peripherals(bus);
        if Bus::read_u32(bus, SPI1 + 0x08).unwrap() & 1 != 0 {
            break;
        }
    }
    let got = Bus::read_u32(bus, SPI1 + 0x0C).unwrap() as u8;
    if let Some(want) = miso {
        assert_eq!(got, want, "MISO for MOSI {mosi:#04x}");
    }
}

/// SPI1 enabled as master with both CS pins push-pull outputs idling high.
/// One frame per bus: under `event-scheduler` a second DR write would sit in
/// the TX buffer behind a frame the bare bus never completes (see [`xfer`]).
fn bus_with_cs_idle_high(log: &Log) -> SystemBus {
    let mut bus = bus_with_two_slaves(log);
    // SSM | SSI | SPE | BR=/2 | MSTR.
    Bus::write_u32(&mut bus, SPI1, 0x0344).unwrap();
    Bus::write_u32(&mut bus, GPIOA + BSRR, 1 << 4).unwrap();
    Bus::write_u32(&mut bus, GPIOB + BSRR, 1 << 12).unwrap();
    Bus::write_u32(&mut bus, GPIOA + CRL, 0x3 << 16).unwrap();
    Bus::write_u32(&mut bus, GPIOB + CRH, 0x3 << 16).unwrap();
    assert_eq!(take(log), Vec::<String>::new(), "idle-high CS is no edge");
    bus
}

#[test]
fn gpio_cs_selects_one_slave_and_delivers_its_edges() {
    // Select B only: A neither sees the byte nor drives MISO.
    let log: Log = Arc::default();
    let mut bus = bus_with_cs_idle_high(&log);
    Bus::write_u32(&mut bus, GPIOB + BSRR, 1 << (12 + 16)).unwrap();
    xfer(&mut bus, 0x5A, Some(0xB2));
    Bus::write_u32(&mut bus, GPIOB + BSRR, 1 << 12).unwrap();
    assert_eq!(take(&log), ["b:select", "b:5a", "b:release"]);

    // A only.
    let mut bus = bus_with_cs_idle_high(&log);
    Bus::write_u32(&mut bus, GPIOA + BSRR, 1 << (4 + 16)).unwrap();
    xfer(&mut bus, 0x3C, Some(0xA1));
    Bus::write_u32(&mut bus, GPIOA + BSRR, 1 << 4).unwrap();
    assert_eq!(take(&log), ["a:select", "a:3c", "a:release"]);

    // Nobody selected: no device is clocked.
    let mut bus = bus_with_cs_idle_high(&log);
    xfer(&mut bus, 0x77, None);
    assert_eq!(take(&log), Vec::<String>::new());
}

/// A CS pad that is not a GPIO output (hardware NSS in AF mode, or simply not
/// configured yet) keeps the device on the bus with no edges — the behaviour
/// every board had before CS routing.
#[test]
fn unconfigured_cs_pad_keeps_the_device_on_the_bus() {
    let log: Log = Arc::default();
    let mut bus = bus_with_two_slaves(&log);
    Bus::write_u32(&mut bus, SPI1, 0x0344).unwrap();
    // PA4 left as a reset-state input; PB12 an output driven high.
    Bus::write_u32(&mut bus, GPIOB + BSRR, 1 << 12).unwrap();
    Bus::write_u32(&mut bus, GPIOB + CRH, 0x3 << 16).unwrap();
    xfer(&mut bus, 0x11, Some(0xA1));
    assert_eq!(take(&log), ["a:11"]);

    // Leaving output mode while selected is a release.
    Bus::write_u32(&mut bus, GPIOB + BSRR, 1 << (12 + 16)).unwrap();
    Bus::write_u32(&mut bus, GPIOB + CRH, 0x4 << 16).unwrap();
    assert_eq!(take(&log), ["b:select", "b:release"]);
}
