// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::network::CanFrame;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
pub struct CanController {
    tx: Sender<CanFrame>,
    rx: Receiver<CanFrame>,

    tx_id: u32,
    tx_data: u32,
    rx_id: u32,
    rx_data: u32,
    rx_pending: bool,
}

impl CanController {
    pub fn new(tx: Sender<CanFrame>, rx: Receiver<CanFrame>) -> Self {
        Self {
            tx,
            rx,
            tx_id: 0,
            tx_data: 0,
            rx_id: 0,
            rx_data: 0,
            rx_pending: false,
        }
    }
}

impl Peripheral for CanController {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let shift = (offset % 4) * 8;

        let val = match reg_offset {
            0x00 => self.tx_id,
            0x04 => self.tx_data,
            0x08 => {
                if self.rx_pending {
                    1
                } else {
                    0
                }
            }
            0x0C => self.rx_id,
            0x10 => self.rx_data,
            _ => 0,
        };
        Ok((val >> shift) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let shift = (offset % 4) * 8;
        let mask = 0xFF << shift;
        let val_shifted = (value as u32) << shift;

        match reg_offset {
            0x00 => self.tx_id = (self.tx_id & !mask) | val_shifted,
            0x04 => self.tx_data = (self.tx_data & !mask) | val_shifted,
            0x08 => {
                if value == 1 {
                    let frame = CanFrame {
                        id: self.tx_id,
                        data: self.tx_data.to_le_bytes().to_vec(),
                    };
                    let _ = self.tx.send(frame);
                } else if value == 2 {
                    self.rx_pending = false;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if !self.rx_pending {
            if let Ok(frame) = self.rx.try_recv() {
                self.rx_id = frame.id;
                let mut data = 0u32;
                for (i, &b) in frame.data.iter().take(4).enumerate() {
                    data |= (b as u32) << (i * 8);
                }
                self.rx_data = data;
                self.rx_pending = true;
            }
        }
        PeripheralTickResult::default()
    }
}
