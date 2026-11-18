// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Digital electrical gate around the existing UART engine. A transceiver
//! supplies direction, polarity, propagation and status; UART serialization,
//! sampling, parity and overrun remain in the shared timed UART implementation.

use std::any::Any;
use std::fmt::Debug;

use crate::network::timed_uart::{LineFormat, TimedUartPort};

pub trait DigitalUartTransceiver: Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn id(&self) -> &str;
    fn driver_delay_ps(&self) -> u64;
    fn receiver_delay_ps(&self) -> u64;
    fn bind_timed_port(&self, port: TimedUartPort);
    fn clock_valid(&self) -> bool {
        true
    }
    /// True only while the physical receiver can present a valid serial
    /// waveform to the MCU, including its enable and filter state. This
    /// also governs local TX echo; a forced idle output is not an echo.
    fn receiver_valid(&self) -> bool {
        self.clock_valid()
    }
    fn allow_tx(&self) -> bool;
    fn accept_peer_byte(&self) -> bool;
    fn allow_timed_tx(&self, cycle: u64, hz: u64) -> bool;
    fn driving_at(&self, cycle: u64, hz: u64) -> bool;
    /// True when the physical transceiver inverts the MCU TX/CQ or CQ/RX
    /// level. MCU CR2 polarity is composed with these, never replaced.
    fn tx_inverted(&self) -> bool;
    fn rx_inverted(&self) -> bool;
    fn record_timed_tx(&self, _cycle: u64, _value: u16, _format: LineFormat, _hz: u64) {}
    fn wire_format(&self, mut format: LineFormat) -> LineFormat {
        format.tx_inverted ^= self.tx_inverted();
        format.rx_inverted ^= self.rx_inverted();
        format
    }
}
