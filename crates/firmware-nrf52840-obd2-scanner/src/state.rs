/// Scanner status bit constants for [`ScannerState::status_flags`].
pub mod flags {
    /// A fresh response has been received.
    pub const CONNECTED: u16 = 1 << 0;
    /// Current readings are not fresh.
    pub const STALE: u16 = 1 << 1;
    /// At least one DTC is present.
    pub const DTC_PRESENT: u16 = 1 << 2;
    /// Latest acquisition ended in a timeout.
    pub const TIMEOUT: u16 = 1 << 3;
    /// A malformed protocol frame was observed.
    pub const MALFORMED: u16 = 1 << 4;
    /// CAN receive buffering overflowed.
    pub const RX_OVERFLOW: u16 = 1 << 5;
    /// CAN controller configuration failed.
    pub const CAN_CONFIG_ERROR: u16 = 1 << 6;
    /// A non-CAN peripheral (OLED/RADIO) failed.
    pub const DEVICE_ERROR: u16 = 1 << 7;
}

/// Validity bits for independently acquired live metrics.
pub mod live {
    pub const RPM: u8 = 1 << 0;
    pub const SPEED: u8 = 1 << 1;
    pub const COOLANT: u8 = 1 << 2;
    pub const ALL: u8 = RPM | SPEED | COOLANT;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquisitionFailure {
    Timeout,
    Malformed,
    Overflow,
    Configuration,
    Device,
}

/// Deterministic PID discovery/live polling schedule.
pub struct PollSchedule {
    discovered: bool,
    live_slot: u8,
}

impl Default for PollSchedule {
    fn default() -> Self {
        Self::new()
    }
}
impl PollSchedule {
    pub const fn new() -> Self {
        Self {
            discovered: false,
            live_slot: 0,
        }
    }
    pub const fn request_pid(&self) -> u8 {
        if !self.discovered {
            0
        } else {
            [0x0c, 0x0d, 0x05][self.live_slot as usize]
        }
    }
    pub fn discovery_failed(&mut self) {}
    pub fn discovery_succeeded(&mut self) {
        self.discovered = true;
        self.live_slot = 0;
    }
    pub fn live_attempted(&mut self) {
        if self.discovered {
            self.live_slot = (self.live_slot + 1) % 3;
        }
    }
}

/// Fixed-size, copyable scanner snapshot shared with later firmware tasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScannerState {
    /// Last decoded engine speed.
    pub rpm: u16,
    /// Last decoded vehicle speed.
    pub speed_kph: u8,
    /// Last decoded coolant temperature.
    pub coolant_c: i16,
    /// Independently acquired fields which contain real ECU samples.
    pub live_valid: u8,
    /// Supported live fields required before the snapshot is connected/fresh.
    pub required_live: u8,
    /// Last decoded non-padding DTC count.
    pub dtc_count: u8,
    /// Bitmask composed from [`flags`].
    pub status_flags: u16,
    /// Wrapping count of fresh samples.
    pub generation: u32,
    /// Saturating scheduler-defined age of the current sample.
    pub sample_age: u16,
    /// Whether `vin` contains a completed VIN.
    pub vin_valid: bool,
    /// Fixed VIN storage.
    pub vin: [u8; 17],
}

impl Default for ScannerState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScannerState {
    /// Creates a stale snapshot with zero readings and no valid VIN.
    pub const fn new() -> Self {
        Self {
            rpm: 0,
            speed_kph: 0,
            coolant_c: 0,
            live_valid: 0,
            required_live: live::ALL,
            dtc_count: 0,
            status_flags: flags::STALE,
            generation: 0,
            sample_age: 0,
            vin_valid: false,
            vin: [0; 17],
        }
    }

    /// Returns true only when every bit in `mask` is set.
    pub const fn has_all(&self, mask: u16) -> bool {
        self.status_flags & mask == mask
    }

    /// Returns true when at least one bit in `mask` is set.
    pub const fn has_any(&self, mask: u16) -> bool {
        self.status_flags & mask != 0
    }

    /// Marks a fresh connected sample, clearing `STALE`/`TIMEOUT` and resetting age.
    pub fn mark_fresh(&mut self) {
        if self.required_live == 0 || self.live_valid & self.required_live != self.required_live {
            return;
        }
        self.status_flags |= flags::CONNECTED;
        self.status_flags &= !(flags::STALE | flags::TIMEOUT);
        self.sample_age = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn set_required_live(&mut self, mask: u8) {
        self.required_live = mask & live::ALL;
        self.refresh_if_complete();
    }

    /// Accepts PID discovery only when all three demo metrics are supported.
    pub fn accept_supported_pids(&mut self, bitmap: u32) -> bool {
        self.required_live = live::ALL;
        let required = (1 << (32 - 0x0c)) | (1 << (32 - 0x0d)) | (1 << (32 - 0x05));
        if bitmap & required != required {
            self.apply_failure(AcquisitionFailure::Malformed);
            false
        } else {
            true
        }
    }

    pub fn record_rpm(&mut self, value: u16) {
        self.rpm = value;
        self.live_valid |= live::RPM;
        self.refresh_if_complete();
    }

    pub fn record_speed(&mut self, value: u8) {
        self.speed_kph = value;
        self.live_valid |= live::SPEED;
        self.refresh_if_complete();
    }

    pub fn record_coolant(&mut self, value: i16) {
        self.coolant_c = value;
        self.live_valid |= live::COOLANT;
        self.refresh_if_complete();
    }

    pub fn invalidate_live(&mut self, mask: u8, failure: AcquisitionFailure) {
        self.live_valid &= !(mask & live::ALL);
        self.apply_failure(failure);
    }

    fn refresh_if_complete(&mut self) {
        if self.required_live != 0 && self.live_valid & self.required_live == self.required_live {
            self.mark_fresh();
        }
    }

    /// Marks timeout/stale and disconnected while retaining readings, DTCs, and VIN.
    pub fn mark_timeout(&mut self) {
        self.status_flags |= flags::TIMEOUT | flags::STALE;
        self.status_flags &= !flags::CONNECTED;
    }

    /// Saturating increment of sample age.
    pub fn increment_age(&mut self) {
        self.sample_age = self.sample_age.saturating_add(1);
    }

    /// Updates DTC count and keeps `DTC_PRESENT` consistent with it.
    pub fn update_dtc_count(&mut self, count: u8) {
        self.dtc_count = count;
        if count == 0 {
            self.status_flags &= !flags::DTC_PRESENT;
        } else {
            self.status_flags |= flags::DTC_PRESENT;
        }
    }

    pub fn clear_dtcs(&mut self) {
        self.update_dtc_count(0);
    }

    /// Stores a completed VIN and marks it valid.
    pub fn set_vin(&mut self, vin: [u8; 17]) {
        self.vin = vin;
        self.vin_valid = true;
    }

    /// Clears VIN validity and zeroes its storage.
    pub fn invalidate_vin(&mut self) {
        self.vin = [0; 17];
        self.vin_valid = false;
    }

    /// Sets only persistent hardware/protocol error bits from `error_flag`.
    pub fn set_error(&mut self, error_flag: u16) {
        self.status_flags |=
            error_flag & (flags::MALFORMED | flags::RX_OVERFLOW | flags::CAN_CONFIG_ERROR);
    }

    pub fn apply_failure(&mut self, failure: AcquisitionFailure) {
        self.status_flags |= flags::STALE;
        self.status_flags &= !flags::CONNECTED;
        match failure {
            AcquisitionFailure::Timeout => self.status_flags |= flags::TIMEOUT,
            AcquisitionFailure::Malformed => self.status_flags |= flags::MALFORMED,
            AcquisitionFailure::Overflow => self.status_flags |= flags::RX_OVERFLOW,
            AcquisitionFailure::Configuration => self.status_flags |= flags::CAN_CONFIG_ERROR,
            AcquisitionFailure::Device => self.status_flags |= flags::DEVICE_ERROR,
        }
    }
}

/// Returns the odd in-progress and following even committed seqlock values.
pub const fn snapshot_sequence_pair(current_even: u32) -> (u32, u32) {
    let odd = current_even.wrapping_add(1) | 1;
    (odd, odd.wrapping_add(1))
}
