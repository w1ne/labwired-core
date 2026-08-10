use crate::obd2::{CanFrame, Error, FLOW_CONTROL_ID, RESPONSE_ID};

const VIN_PAYLOAD_LEN: usize = 20;

/// Result of accepting one ISO-TP frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsoTpEvent {
    /// Caller must transmit this flow-control frame.
    FlowControl(CanFrame),
    /// Transfer is valid but not complete.
    Pending,
    /// Complete, exactly 17-byte VIN.
    Complete([u8; 17]),
}

/// Heap-free receiver for the deterministic Mode 09 VIN wire format used by Task 6.
///
/// The ECU sends on CAN ID `0x7E8`; flow control is returned on physical ID `0x7E0`
/// as `[0x30, 0, 0, 0, 0, 0, 0, 0]`. The First Frame PCI declares exactly 20
/// application bytes: `[0x49, 0x02, 0x01]` followed by the 17-byte VIN. That FF
/// carries the three-byte application header and the first three VIN bytes. CF1
/// (sequence 1) carries the next seven VIN bytes and CF2 (sequence 2) carries the
/// final seven. Both consecutive frames therefore have a CAN DLC of eight. A
/// Mode 09 negative response is accepted only as the unpadded single frame
/// `[0x03, 0x7F, 0x09, NRC]` with DLC 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VinReassembler {
    payload: [u8; VIN_PAYLOAD_LEN],
    received: u8,
    next_sequence: u8,
    active: bool,
}

impl Default for VinReassembler {
    fn default() -> Self {
        Self::new()
    }
}

impl VinReassembler {
    /// Creates an idle receiver with zeroed storage.
    pub const fn new() -> Self {
        Self {
            payload: [0; VIN_PAYLOAD_LEN],
            received: 0,
            next_sequence: 1,
            active: false,
        }
    }

    /// Abandons any transfer and zeroes all buffered bytes.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Ends an active transfer as [`Error::Incomplete`], resetting its storage.
    pub fn timeout(&mut self) -> Result<(), Error> {
        if self.active {
            self.reset();
            Err(Error::Incomplete)
        } else {
            Ok(())
        }
    }

    /// Accepts one ECU response frame; every error resets and zeroes receiver state.
    pub fn push(&mut self, frame: &CanFrame) -> Result<IsoTpEvent, Error> {
        let result = self.push_inner(frame);
        if result.is_err() {
            self.reset();
        }
        result
    }

    fn push_inner(&mut self, frame: &CanFrame) -> Result<IsoTpEvent, Error> {
        if frame.len > 8 {
            return Err(Error::InvalidLength);
        }
        if frame.id != RESPONSE_ID {
            return Err(Error::WrongId);
        }
        if frame.len == 0 {
            return Err(Error::InvalidLength);
        }
        match frame.data[0] >> 4 {
            0 => self.single_frame(frame),
            1 => self.first_frame(frame),
            2 => self.consecutive_frame(frame),
            _ => Err(Error::UnexpectedFrame),
        }
    }

    fn single_frame(&mut self, frame: &CanFrame) -> Result<IsoTpEvent, Error> {
        if self.active {
            return Err(Error::UnexpectedFrame);
        }
        let declared = usize::from(frame.data[0] & 0x0f);
        if frame.len != 4 || usize::from(frame.len) != declared + 1 || declared != 3 {
            return Err(Error::InvalidLength);
        }
        if frame.data[1] != 0x7f {
            return Err(Error::UnexpectedFrame);
        }
        if frame.data[2] != 9 {
            return Err(Error::UnsupportedService);
        }
        Err(Error::NegativeResponse {
            service: 9,
            nrc: frame.data[3],
        })
    }

    fn first_frame(&mut self, frame: &CanFrame) -> Result<IsoTpEvent, Error> {
        if self.active {
            return Err(Error::UnexpectedFrame);
        }
        if frame.len < 8 {
            return Err(Error::ShortPayload);
        }
        let total = (usize::from(frame.data[0] & 0x0f) << 8) | usize::from(frame.data[1]);
        if total > VIN_PAYLOAD_LEN {
            return Err(Error::Oversize);
        }
        // Deterministic ECU format: 49 02 01, then exactly 17 VIN bytes.
        if total != VIN_PAYLOAD_LEN {
            return Err(Error::Malformed);
        }
        if frame.data[2] != 0x49 {
            return Err(Error::UnsupportedService);
        }
        if frame.data[3] != 2 {
            return Err(Error::UnsupportedPid);
        }
        if frame.data[4] != 1 {
            return Err(Error::Malformed);
        }
        self.payload[..6].copy_from_slice(&frame.data[2..8]);
        self.received = 6;
        self.next_sequence = 1;
        self.active = true;
        Ok(IsoTpEvent::FlowControl(CanFrame {
            id: FLOW_CONTROL_ID,
            len: 8,
            data: [0x30, 0, 0, 0, 0, 0, 0, 0],
        }))
    }

    fn consecutive_frame(&mut self, frame: &CanFrame) -> Result<IsoTpEvent, Error> {
        if !self.active {
            return Err(Error::UnexpectedFrame);
        }
        if frame.len != 8 {
            return Err(Error::InvalidLength);
        }
        if frame.data[0] & 0x0f != self.next_sequence {
            return Err(Error::Sequence);
        }
        let remaining = VIN_PAYLOAD_LEN - usize::from(self.received);
        let chunk = remaining.min(7);
        if usize::from(frame.len) < chunk + 1 {
            return Err(Error::ShortPayload);
        }
        let start = usize::from(self.received);
        self.payload[start..start + chunk].copy_from_slice(&frame.data[1..1 + chunk]);
        self.received += chunk as u8;
        self.next_sequence = (self.next_sequence + 1) & 0x0f;
        if usize::from(self.received) != VIN_PAYLOAD_LEN {
            return Ok(IsoTpEvent::Pending);
        }
        if self.payload[..3] != [0x49, 0x02, 0x01] {
            return Err(if self.payload[0] != 0x49 {
                Error::UnsupportedService
            } else if self.payload[1] != 2 {
                Error::UnsupportedPid
            } else {
                Error::Malformed
            });
        }
        let mut vin = [0; 17];
        vin.copy_from_slice(&self.payload[3..]);
        self.reset();
        Ok(IsoTpEvent::Complete(vin))
    }
}
