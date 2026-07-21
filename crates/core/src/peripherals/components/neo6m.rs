// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::uart::UartStreamDevice;
use std::any::Any;
use std::collections::VecDeque;

/// Simulated NEO-6M GPS module.
///
/// Emits NMEA sentences (GGA + RMC, alternating) at 2 Hz over the UART RX path.
/// All domain logic — NMEA formatting, DDMM.mmmm conversion, checksum, pacing — lives here in
/// Rust core. The WASM bridge and UI are thin pass-throughs.
#[derive(Debug, serde::Serialize)]
pub struct Neo6mGps {
    /// Current latitude in decimal degrees (positive = North).
    latitude_deg: f64,
    /// Current longitude in decimal degrees (positive = East).
    longitude_deg: f64,
    /// 'A' = active fix, 'V' = void (no fix).
    fix_status: char,
    /// Internal byte queue — sentences are pre-rendered here and drained per poll.
    #[serde(skip)]
    out_queue: VecDeque<u8>,
    /// Microseconds accumulated since last sentence generation. 1 Hz GPS → 1_000_000 us.
    time_since_last_sentence_us: u32,
    /// Round-robin sentence type counter so we emit GGA, RMC alternately.
    sentence_index: u8,
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Neo6mGps {
    fn default() -> Self {
        Self::new()
    }
}

impl Neo6mGps {
    /// Create a new GPS device. Defaults to San Francisco (recognizable lat/lon for demo).
    pub fn new() -> Self {
        Self {
            latitude_deg: 37.7749,
            longitude_deg: -122.4194,
            fix_status: 'A',
            out_queue: VecDeque::new(),
            time_since_last_sentence_us: 0,
            sentence_index: 0,
            component_id: None,
        }
    }

    /// Set the simulated GPS position.
    pub fn set_position(&mut self, lat_deg: f64, lon_deg: f64) {
        self.latitude_deg = lat_deg;
        self.longitude_deg = lon_deg;
    }

    /// Enable or disable the GPS fix.
    pub fn set_fix(&mut self, active: bool) {
        self.fix_status = if active { 'A' } else { 'V' };
    }

    /// Returns the current simulated position as (lat_deg, lon_deg).
    pub fn position(&self) -> (f64, f64) {
        (self.latitude_deg, self.longitude_deg)
    }

    /// Returns true when the GPS has an active fix.
    pub fn has_fix(&self) -> bool {
        self.fix_status == 'A'
    }

    /// Emit one NMEA sentence into the out_queue. Round-robins between GGA and RMC.
    ///
    /// NMEA sentence format:
    ///   $<payload>*<XX>\r\n
    /// where XX is the XOR checksum of every byte between '$' and '*' (exclusive).
    fn enqueue_next_sentence(&mut self) {
        let (lat_dm, lat_hemi) = degrees_to_nmea(self.latitude_deg, true);
        let (lon_dm, lon_hemi) = degrees_to_nmea(self.longitude_deg, false);

        let payload = match self.sentence_index % 2 {
            0 => {
                // GGA: Global Positioning System Fix Data
                // $GPGGA,hhmmss.ss,llll.llll,a,yyyyy.yyyy,a,q,nn,h.h,alt,M,sep,M,,*CS
                format!(
                    "GPGGA,120000.00,{:09.4},{},{:010.4},{},{},08,1.0,10.0,M,0.0,M,,",
                    lat_dm,
                    lat_hemi,
                    lon_dm,
                    lon_hemi,
                    if self.fix_status == 'A' { 1 } else { 0 }
                )
            }
            _ => {
                // RMC: Recommended Minimum Specific GPS Data
                // $GPRMC,hhmmss.ss,A/V,llll.llll,a,yyyyy.yyyy,a,sog,cog,ddmmyy,,,A*CS
                format!(
                    "GPRMC,120000.00,{},{:09.4},{},{:010.4},{},0.0,0.0,150526,,,A",
                    self.fix_status, lat_dm, lat_hemi, lon_dm, lon_hemi
                )
            }
        };
        self.sentence_index = self.sentence_index.wrapping_add(1);

        // Compute XOR checksum of the payload bytes.
        let checksum = payload.bytes().fold(0u8, |acc, b| acc ^ b);
        let sentence = format!("${}*{:02X}\r\n", payload, checksum);

        for byte in sentence.bytes() {
            self.out_queue.push_back(byte);
        }
    }
}

/// Convert decimal degrees to NMEA DDMM.mmmm format + hemisphere character.
///
/// NMEA uses degrees + decimal minutes (not decimal degrees), so:
///   lat  37.7749° → 3746.4940 N
///   lon -122.4194° → 12225.1640 W
fn degrees_to_nmea(deg: f64, is_latitude: bool) -> (f64, char) {
    let abs = deg.abs();
    let degrees = abs.floor();
    let minutes = (abs - degrees) * 60.0;
    let dm = degrees * 100.0 + minutes;
    let hemi = if is_latitude {
        if deg >= 0.0 {
            'N'
        } else {
            'S'
        }
    } else {
        if deg >= 0.0 {
            'E'
        } else {
            'W'
        }
    };
    (dm, hemi)
}

impl UartStreamDevice for Neo6mGps {
    fn poll(&mut self, elapsed_us: u32) -> Option<u8> {
        // Drain any pre-queued bytes first — one byte per poll call models a UART byte stream.
        if let Some(b) = self.out_queue.pop_front() {
            return Some(b);
        }
        // Accumulate time; emit a new sentence every ~500 ms (GGA + RMC alternating = 2 Hz pair).
        self.time_since_last_sentence_us =
            self.time_since_last_sentence_us.saturating_add(elapsed_us);
        if self.time_since_last_sentence_us >= 500_000 {
            self.time_since_last_sentence_us = 0;
            self.enqueue_next_sentence();
            return self.out_queue.pop_front();
        }
        None
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        Some(self)
    }
}

/// Drivable position and fix state. Driving `lat` preserves `lon` and vice
/// versa; `fix` is 0 = no fix (status V), 1 = active fix (status A). One
/// table backs BOTH the `SimInput` impl and the kit metadata, so the device
/// schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "lat",
        label: "Latitude",
        unit: "°",
        min: -90.0,
        max: 90.0,
    },
    crate::sim_input::InputChannel {
        key: "lon",
        label: "Longitude",
        unit: "°",
        min: -180.0,
        max: 180.0,
    },
    crate::sim_input::InputChannel {
        key: "fix",
        label: "Fix",
        unit: "level",
        min: 0.0,
        max: 1.0,
    },
];

impl crate::sim_input::SimInput for Neo6mGps {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        let (lat, lon) = self.position();
        match key {
            "lat" => self.set_position(value, lon),
            "lon" => self.set_position(lat, value),
            "fix" => self.set_fix(value >= 0.5),
            _ => unreachable!("require_channel validated the key"),
        }
        Ok(())
    }
    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct Neo6mGpsKit;
pub static NEO6M_KIT: Neo6mGpsKit = Neo6mGpsKit;

static NEO6M_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "neo6m-gps",
    label: "NEO-6M GPS",
    summary: "GPS module streaming NMEA sentences over UART RX.",
    detail: "GGA + RMC sentences with XOR checksum, generated entirely in the Rust core. \
             Firmware echoes the stream back to the host UART.",
    transport: Transport::Uart,
    category: Category::Uart,
    config_keys: &[
        ConfigKey {
            name: "lat_deg",
            ty: ConfigType::Float,
            doc: "Initial latitude in decimal degrees (paired with lon_deg).",
        },
        ConfigKey {
            name: "lon_deg",
            ty: ConfigType::Float,
            doc: "Initial longitude in decimal degrees (paired with lat_deg).",
        },
    ],
    labs: &[LabRef {
        board_id: "neo6m-gps-lab",
        chip: "stm32f103",
        example_dir: "neo6m-gps-lab",
        demo_elf: "demo-neo6m-gps-lab.elf",
    }],
};

impl PeripheralKit for Neo6mGpsKit {
    fn metadata(&self) -> &'static KitMetadata {
        &NEO6M_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let lat = ctx.config_f64("lat_deg");
        let lon = ctx.config_f64("lon_deg");

        let mut gps = Neo6mGps::new();
        if let (Some(lat), Some(lon)) = (lat, lon) {
            gps.set_position(lat, lon);
        }
        crate::sim_input::SimInput::set_component_id(&mut gps, ctx.device_id().to_string());
        let uart = ctx.uart()?;
        uart.attach_stream(Box::new(gps));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nmea_checksum_gga() {
        let mut gps = Neo6mGps::new();
        gps.sentence_index = 0; // force GGA next
        gps.enqueue_next_sentence();
        let sentence: String = gps.out_queue.iter().map(|&b| b as char).collect();
        // Sentence should start with $ and contain *XX checksum
        assert!(sentence.starts_with('$'), "sentence must start with $");
        assert!(sentence.contains('*'), "sentence must contain *");
        // Verify checksum
        let star_pos = sentence.find('*').unwrap();
        let payload = &sentence[1..star_pos];
        let expected_cs = payload.bytes().fold(0u8, |a, b| a ^ b);
        let cs_str = &sentence[star_pos + 1..star_pos + 3];
        let actual_cs = u8::from_str_radix(cs_str, 16).unwrap();
        assert_eq!(expected_cs, actual_cs, "checksum mismatch");
    }

    #[test]
    fn test_nmea_checksum_rmc() {
        let mut gps = Neo6mGps::new();
        gps.sentence_index = 1; // force RMC next
        gps.enqueue_next_sentence();
        let sentence: String = gps.out_queue.iter().map(|&b| b as char).collect();
        assert!(sentence.starts_with("$GPRMC"), "must be RMC");
        let star_pos = sentence.find('*').unwrap();
        let payload = &sentence[1..star_pos];
        let expected_cs = payload.bytes().fold(0u8, |a, b| a ^ b);
        let cs_str = &sentence[star_pos + 1..star_pos + 3];
        let actual_cs = u8::from_str_radix(cs_str, 16).unwrap();
        assert_eq!(expected_cs, actual_cs, "checksum mismatch in RMC");
    }

    #[test]
    fn test_degrees_to_nmea_sf() {
        let (dm, hemi) = degrees_to_nmea(37.7749, true);
        // 37° + 0.7749*60 = 37° 46.494' → DDMM.mmmm = 3746.4940
        assert!((dm - 3746.494).abs() < 0.001);
        assert_eq!(hemi, 'N');

        let (dm2, hemi2) = degrees_to_nmea(-122.4194, false);
        // 122° + 0.4194*60 = 122° 25.164' → DDMM.mmmm = 12225.1640
        assert!((dm2 - 12225.164).abs() < 0.001);
        assert_eq!(hemi2, 'W');
    }

    #[test]
    fn test_poll_emits_bytes_after_500ms() {
        let mut gps = Neo6mGps::new();
        // Before 500 ms, should be silent
        let byte = gps.poll(100_000); // 100 ms
        assert!(byte.is_none(), "should not emit before 500 ms");
        // After 500 ms total should emit
        let byte = gps.poll(400_000); // 400 ms more → 500 ms total
        assert!(byte.is_some(), "should emit after 500 ms");
    }

    #[test]
    fn test_set_position_affects_sentence() {
        let mut gps = Neo6mGps::new();
        gps.set_position(51.5074, -0.1278); // London
        gps.sentence_index = 0;
        gps.enqueue_next_sentence();
        let sentence: String = gps.out_queue.iter().map(|&b| b as char).collect();
        assert!(sentence.contains("GPGGA"), "should be GGA");
        assert!(sentence.contains('N'), "London is north");
        assert!(sentence.contains('W'), "London is west");
    }

    #[test]
    fn test_void_fix_in_rmc() {
        let mut gps = Neo6mGps::new();
        gps.set_fix(false);
        gps.sentence_index = 1; // RMC
        gps.enqueue_next_sentence();
        let sentence: String = gps.out_queue.iter().map(|&b| b as char).collect();
        assert!(
            sentence.contains(",V,"),
            "void fix must show 'V' in RMC status field"
        );
    }
}
