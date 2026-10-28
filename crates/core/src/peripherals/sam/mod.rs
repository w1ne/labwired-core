// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Microchip **SAM** family peripherals (SAM D21 / D51 / E5x).
//!
//! The PORT model lives with the other GPIO families in
//! [`crate::peripherals::gpio`] (`GpioRegisterLayout::SamPort`), because it is
//! a register layout of the shared port model rather than a peripheral of its
//! own — that is what gets a SAM pad the pad-line routing, the logic-analyzer
//! tap and the `board_io` button plumbing for free.
//!
//! What is here is the silicon that has no counterpart elsewhere in the tree:
//! SERCOM, the one block that *is* the UART, the SPI controller and the I²C
//! controller depending on `CTRLA.MODE`.

pub mod adc;
pub mod clock_tree;
pub mod cmcc;
pub mod dac;
pub mod dmac_byte;
pub mod eic;
pub mod nvmctrl;
pub mod oscctrl;
pub mod pac;
pub mod sercom_i2cm;
pub mod sercom_spi;
pub mod sercom_usart;
pub mod tc;
pub mod tcc;
pub mod usb_device;
