// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::network::WirelessPacket;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
pub struct RadioController {
    tx: Sender<WirelessPacket>,
    rx: Receiver<WirelessPacket>,

    tx_channel: u8,
    tx_power: u8,

    rx_pending: bool,
    rx_packet: Option<WirelessPacket>,

    // Register shadows
    #[allow(dead_code)]
    reg_rx_id: u32, // placeholder if we add IDs
    reg_rx_data: u32,
    reg_rx_channel: u8,
}

impl RadioController {
    pub fn new(tx: Sender<WirelessPacket>, rx: Receiver<WirelessPacket>) -> Self {
        Self {
            tx,
            rx,
            tx_channel: 0,
            tx_power: 0,
            rx_pending: false,
            rx_packet: None,
            reg_rx_id: 0,
            reg_rx_data: 0,
            reg_rx_channel: 0,
        }
    }
}

impl Peripheral for RadioController {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let shift = (offset % 4) * 8;

        let val = match reg_offset {
            0x00 => self.tx_channel as u32,
            0x04 => self.tx_power as u32,
            0x08 => 0, // TX Trigger is WO
            0x0C => {
                if self.rx_pending {
                    1
                } else {
                    0
                }
            }
            0x10 => self.reg_rx_channel as u32,
            0x14 => self.reg_rx_data,
            _ => 0,
        };
        Ok((val >> shift) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let shift = (offset % 4) * 8;
        let _mask = 0xFF << shift;
        // let val_shifted = (value as u32) << shift;

        match reg_offset {
            0x00 => {
                if shift == 0 {
                    self.tx_channel = value
                }
            }
            0x04 => {
                if shift == 0 {
                    self.tx_power = value
                }
            }
            0x08 => {
                if value == 1 {
                    // Trigger TX
                    let packet = WirelessPacket {
                        channel: self.tx_channel,
                        payload: vec![0, 0, 0, 0], // Placeholder payload for now
                    };
                    let _ = self.tx.send(packet);
                } else if value == 2 {
                    // Clear RX
                    self.rx_pending = false;
                    self.rx_packet = None;
                }
            }
            0x14 => {
                // If we want to allow setting TX data via this register too?
                // For now, let's keep it simple.
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if !self.rx_pending {
            if let Ok(packet) = self.rx.try_recv() {
                // Simple channel filtering
                if packet.channel == self.tx_channel {
                    self.reg_rx_channel = packet.channel;
                    let mut data = 0u32;
                    for (i, &b) in packet.payload.iter().take(4).enumerate() {
                        data |= (b as u32) << (i * 8);
                    }
                    self.reg_rx_data = data;
                    self.rx_pending = true;
                    self.rx_packet = Some(packet);
                }
            }
        }
        PeripheralTickResult::default()
    }
}
