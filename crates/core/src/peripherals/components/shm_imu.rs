// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Shared-memory I2C register bridge.
//!
//! This small device model is used by Project Phoenix ProximityAgent tests. The
//! external harness mutates a byte-addressed shared-memory file while firmware
//! accesses the same bytes through I2C register-pointer transactions.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use crate::peripherals::i2c::I2cDevice;

const DEFAULT_ADDR: u8 = 0x24;
const DEFAULT_SIZE: usize = 128;

#[derive(Debug)]
pub struct ShmImu {
    address: u8,
    shm_path: PathBuf,
    size: usize,
    current_register: u8,
    register_address_written: bool,
}

impl ShmImu {
    pub fn new(address: u8, shm_path: PathBuf, size: usize) -> Self {
        Self {
            address,
            shm_path,
            size: size.max(1),
            current_register: 0,
            register_address_written: false,
        }
    }

    pub fn proximity_default(shm_path: PathBuf) -> Self {
        Self::new(DEFAULT_ADDR, shm_path, DEFAULT_SIZE)
    }

    fn read_byte(&self, offset: u8) -> u8 {
        if offset as usize >= self.size {
            return 0;
        }
        let Ok(mut file) = OpenOptions::new().read(true).open(&self.shm_path) else {
            return 0;
        };
        if file.seek(SeekFrom::Start(offset as u64)).is_err() {
            return 0;
        }
        let mut byte = [0_u8; 1];
        if file.read_exact(&mut byte).is_ok() {
            byte[0]
        } else {
            0
        }
    }

    fn write_byte(&self, offset: u8, value: u8) {
        if offset as usize >= self.size {
            return;
        }
        let Ok(mut file) = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&self.shm_path)
        else {
            return;
        };
        if file.metadata().map(|m| m.len()).unwrap_or(0) < self.size as u64 {
            let _ = file.set_len(self.size as u64);
        }
        if file.seek(SeekFrom::Start(offset as u64)).is_ok() {
            let _ = file.write_all(&[value]);
            let _ = file.flush();
        }
    }
}

impl I2cDevice for ShmImu {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let value = self.read_byte(self.current_register);
        self.current_register = self.current_register.wrapping_add(1);
        value
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            self.current_register = data;
            self.register_address_written = true;
        } else {
            self.write_byte(self.current_register, data);
            self.current_register = self.current_register.wrapping_add(1);
        }
    }

    fn stop(&mut self) {
        self.register_address_written = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_read_and_write_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "labwired_shm_imu_test_{}",
            std::process::id()
        ));
        std::fs::write(&path, vec![0_u8; 8]).unwrap();
        let mut dev = ShmImu::new(0x24, path.clone(), 8);

        dev.start();
        dev.write(0x01);
        dev.write(0x12);
        dev.write(0x34);
        dev.stop();

        dev.start();
        dev.write(0x01);
        assert_eq!(dev.read(), 0x12);
        assert_eq!(dev.read(), 0x34);

        let _ = std::fs::remove_file(path);
    }
}
