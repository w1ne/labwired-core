/// Functional broadcast request CAN identifier.
pub const REQUEST_ID: u16 = 0x7df;
/// Physical flow-control CAN identifier for the selected ECU.
pub const FLOW_CONTROL_ID: u16 = 0x7e0;
/// Response CAN identifier accepted from the selected ECU.
pub const RESPONSE_ID: u16 = 0x7e8;

/// Classic CAN frame used at OBD-II protocol boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanFrame {
    /// 11-bit CAN identifier.
    pub id: u16,
    /// Data length code; values above eight are rejected.
    pub len: u8,
    /// Fixed backing storage; only bytes below `len` are on the wire.
    pub data: [u8; 8],
}

/// Compact protocol and reassembly failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Frame/DLC or declared payload length is invalid.
    InvalidLength,
    /// Frame came from an unexpected CAN identifier.
    WrongId,
    /// Payload structure or a fixed-format field is invalid.
    Malformed,
    /// Declared payload is shorter than the requested value needs.
    ShortPayload,
    /// ECU returned `0x7F`, including the echoed service and NRC.
    NegativeResponse { service: u8, nrc: u8 },
    /// Response did not contain the expected positive service.
    UnsupportedService,
    /// Response did not echo the requested PID.
    UnsupportedPid,
    /// ISO-TP consecutive-frame sequence was skipped or repeated.
    Sequence,
    /// ISO-TP frame type is invalid for the current receiver state.
    UnexpectedFrame,
    /// ISO-TP transfer cannot fit the fixed VIN buffer.
    Oversize,
    /// Active ISO-TP transfer ended before completion.
    Incomplete,
}

/// Builds a Mode 01 PID request padded to eight CAN bytes.
pub const fn mode01_request(pid: u8) -> CanFrame {
    request([2, 1, pid, 0, 0, 0, 0, 0])
}

/// Builds a Mode 03 request to read stored DTCs.
pub const fn read_dtcs_request() -> CanFrame {
    request([1, 3, 0, 0, 0, 0, 0, 0])
}

/// Builds a Mode 04 request to clear emissions DTCs.
pub const fn clear_dtcs_request() -> CanFrame {
    request([1, 4, 0, 0, 0, 0, 0, 0])
}

/// Builds the Mode 09 PID 02 VIN request.
pub const fn vin_request() -> CanFrame {
    request([2, 9, 2, 0, 0, 0, 0, 0])
}

const fn request(data: [u8; 8]) -> CanFrame {
    CanFrame {
        id: REQUEST_ID,
        len: 8,
        data,
    }
}

fn single_frame_payload(frame: &CanFrame) -> Result<&[u8], Error> {
    if frame.len > 8 {
        return Err(Error::InvalidLength);
    }
    if frame.id != RESPONSE_ID {
        return Err(Error::WrongId);
    }
    if frame.len == 0 || frame.data[0] & 0xf0 != 0 {
        return Err(Error::Malformed);
    }
    let payload_len = usize::from(frame.data[0] & 0x0f);
    if payload_len == 0 || payload_len + 1 > usize::from(frame.len) {
        return Err(Error::ShortPayload);
    }
    Ok(&frame.data[1..=payload_len])
}

fn positive_pid_payload(
    frame: &CanFrame,
    service: u8,
    pid: u8,
    expected: usize,
) -> Result<&[u8], Error> {
    let payload = single_frame_payload(frame)?;
    check_negative(payload, service)?;
    if payload[0] != service.wrapping_add(0x40) {
        return Err(Error::UnsupportedService);
    }
    if payload.len() < 2 {
        return Err(Error::ShortPayload);
    }
    if payload[1] != pid {
        return Err(Error::UnsupportedPid);
    }
    if payload.len() < expected {
        return Err(Error::ShortPayload);
    }
    if payload.len() > expected {
        return Err(Error::InvalidLength);
    }
    Ok(payload)
}

fn check_negative(payload: &[u8], requested_service: u8) -> Result<(), Error> {
    if payload[0] == 0x7f {
        if payload.len() < 3 {
            return Err(Error::ShortPayload);
        }
        if payload[1] != requested_service {
            return Err(Error::UnsupportedService);
        }
        if payload.len() != 3 {
            return Err(Error::InvalidLength);
        }
        return Err(Error::NegativeResponse {
            service: payload[1],
            nrc: payload[2],
        });
    }
    Ok(())
}

/// Decodes the Mode 01 PID 00 supported-PID bitmap.
pub fn decode_supported_pids(frame: &CanFrame) -> Result<u32, Error> {
    let p = positive_pid_payload(frame, 1, 0, 6)?;
    Ok(u32::from_be_bytes([p[2], p[3], p[4], p[5]]))
}

/// Decodes Mode 01 PID 0C as `(256*A+B)/4` RPM.
pub fn decode_rpm(frame: &CanFrame) -> Result<u16, Error> {
    let p = positive_pid_payload(frame, 1, 0x0c, 4)?;
    Ok((u16::from(p[2]) * 256 + u16::from(p[3])) / 4)
}

/// Decodes Mode 01 PID 0D vehicle speed in km/h.
pub fn decode_speed(frame: &CanFrame) -> Result<u8, Error> {
    Ok(positive_pid_payload(frame, 1, 0x0d, 3)?[2])
}

/// Decodes Mode 01 PID 05 coolant temperature in degrees Celsius.
pub fn decode_coolant(frame: &CanFrame) -> Result<i16, Error> {
    Ok(i16::from(positive_pid_payload(frame, 1, 5, 3)?[2]) - 40)
}

/// SAE diagnostic trouble-code system prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DtcSystem {
    /// `P` powertrain code.
    Powertrain,
    /// `C` chassis code.
    Chassis,
    /// `B` body code.
    Body,
    /// `U` network code.
    Network,
}

/// Allocation-free SAE DTC split into its system and four hexadecimal digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dtc {
    /// Leading SAE system letter.
    pub system: DtcSystem,
    /// Four numeric nibbles, each in `0..=15`.
    pub digits: [u8; 4],
}

impl Dtc {
    /// Returns the five ASCII bytes, using uppercase `A` through `F` for hex digits.
    pub const fn ascii(self) -> [u8; 5] {
        let system = match self.system {
            DtcSystem::Powertrain => b'P',
            DtcSystem::Chassis => b'C',
            DtcSystem::Body => b'B',
            DtcSystem::Network => b'U',
        };
        [
            system,
            hex_ascii(self.digits[0]),
            hex_ascii(self.digits[1]),
            hex_ascii(self.digits[2]),
            hex_ascii(self.digits[3]),
        ]
    }

    const fn from_raw(raw: u16) -> Self {
        let system = match raw >> 14 {
            0 => DtcSystem::Powertrain,
            1 => DtcSystem::Chassis,
            2 => DtcSystem::Body,
            _ => DtcSystem::Network,
        };
        Self {
            system,
            digits: [
                ((raw >> 12) & 3) as u8,
                ((raw >> 8) & 0x0f) as u8,
                ((raw >> 4) & 0x0f) as u8,
                (raw & 0x0f) as u8,
            ],
        }
    }
}

const fn hex_ascii(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0' + nibble
    } else {
        b'A' + (nibble - 10)
    }
}

const EMPTY_DTC: Dtc = Dtc {
    system: DtcSystem::Powertrain,
    digits: [0; 4],
};

/// Up to three non-padding DTCs from one classic-CAN Mode 03 response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DtcList {
    /// Fixed DTC storage; entries at or above `count` are unspecified placeholders.
    pub dtcs: [Dtc; 3],
    /// Number of valid entries in `dtcs`.
    pub count: u8,
}

/// Decodes one Mode 03 single-frame response and ignores `0x0000` padding.
pub fn decode_dtcs(frame: &CanFrame) -> Result<DtcList, Error> {
    let payload = single_frame_payload(frame)?;
    check_negative(payload, 3)?;
    if payload[0] != 0x43 {
        return Err(Error::UnsupportedService);
    }
    if (payload.len() - 1) % 2 != 0 {
        return Err(Error::Malformed);
    }
    let mut result = DtcList {
        dtcs: [EMPTY_DTC; 3],
        count: 0,
    };
    for pair in payload[1..].chunks_exact(2) {
        let raw = u16::from_be_bytes([pair[0], pair[1]]);
        if raw != 0 {
            result.dtcs[usize::from(result.count)] = Dtc::from_raw(raw);
            result.count += 1;
        }
    }
    Ok(result)
}

/// Validates a Mode 04 positive response.
pub fn decode_clear_dtcs(frame: &CanFrame) -> Result<(), Error> {
    let payload = single_frame_payload(frame)?;
    check_negative(payload, 4)?;
    if payload[0] != 0x44 {
        return Err(Error::UnsupportedService);
    }
    if payload.len() != 1 {
        return Err(Error::Malformed);
    }
    Ok(())
}
