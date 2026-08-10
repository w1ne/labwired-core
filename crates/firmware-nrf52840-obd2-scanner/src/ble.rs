//! Fixed BLE telemetry encoding and the nRF52840 raw RADIO transmitter.
//!
//! This uses the repository's Air Tracer contract: a BLE-1M-shaped raw packet,
//! not a complete standards-compliant GAP advertising PDU.

use core::{
    cell::UnsafeCell,
    ptr::{read_volatile, write_volatile},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{flags, ScannerState};

pub const PAYLOAD_LEN: usize = 9;
pub const VERSION: u8 = 1;
pub const RADIO_DMA_STATIC: bool = true;
/// All currently meaningful state flags have stable, identical on-wire bits.
pub const WIRE_FLAGS: u16 = flags::CONNECTED
    | flags::STALE
    | flags::DTC_PRESENT
    | flags::TIMEOUT
    | flags::MALFORMED
    | flags::RX_OVERFLOW
    | flags::CAN_CONFIG_ERROR
    | flags::DEVICE_ERROR;

/// Layout: version, flags, RPM LE, speed, decoded coolant Celsius, DTC count,
/// generation LE16. Values outside the one-byte Celsius range clamp to 0..255.
pub fn encode_manufacturer_payload(state: &ScannerState) -> [u8; PAYLOAD_LEN] {
    debug_assert_eq!(WIRE_FLAGS & !0xff, 0);
    let coolant = state.coolant_c.clamp(0, 255) as u8;
    let rpm = state.rpm.to_le_bytes();
    let generation = (state.generation as u16).to_le_bytes();
    [
        VERSION,
        (state.status_flags & WIRE_FLAGS) as u8,
        rpm[0],
        rpm[1],
        state.speed_kph,
        coolant,
        state.dtc_count,
        generation[0],
        generation[1],
    ]
}

const CLOCK: usize = 0x4000_0000;
const RADIO: usize = 0x4000_1000;
const WAIT_LIMIT: u32 = 200_000;

pub struct Radio {
    stuck: bool,
}

struct PacketCell(UnsafeCell<[u8; PAYLOAD_LEN + 2]>);
unsafe impl Sync for PacketCell {}
static PACKET: PacketCell = PacketCell(UnsafeCell::new([0; PAYLOAD_LEN + 2]));
static TAKEN: AtomicBool = AtomicBool::new(false);

impl Radio {
    /// Claims the sole non-reentrant RADIO/static packet-buffer instance.
    pub fn take() -> Option<Self> {
        TAKEN
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self { stuck: false })
    }

    pub fn init(&mut self) -> bool {
        unsafe {
            wr(CLOCK, 0x100, 0);
            wr(CLOCK, 0x000, 1);
            if !wait_set(CLOCK, 0x100) {
                return false;
            }
            packet_set(0, 0xab);
            packet_set(1, PAYLOAD_LEN as u8);
            wr(RADIO, 0x510, 3);
            wr(RADIO, 0x508, 42);
            wr(RADIO, 0x514, 8 | (1 << 8));
            wr(RADIO, 0x518, 0xff | (1 << 25));
            wr(RADIO, 0x51c, 0xcafe_ba00);
            wr(RADIO, 0x524, 0xbe);
            wr(RADIO, 0x52c, 0);
            wr(RADIO, 0x534, 3);
            wr(RADIO, 0x538, 0x065b);
            wr(RADIO, 0x53c, 0x55_5555);
            wr(RADIO, 0x554, 42);
            wr(RADIO, 0x504, PACKET.0.get() as *mut u8 as u32);
        }
        true
    }

    pub fn transmit(&mut self, payload: &[u8; PAYLOAD_LEN]) -> bool {
        if self.stuck {
            return false;
        }
        for (index, byte) in payload.iter().enumerate() {
            packet_set(2 + index, *byte);
        }
        unsafe {
            wr(RADIO, 0x504, PACKET.0.get() as *mut u8 as u32);
            wr(RADIO, 0x100, 0);
            wr(RADIO, 0x000, 1);
            if !wait_set(RADIO, 0x100) {
                return self.abort();
            }
            wr(RADIO, 0x10c, 0);
            wr(RADIO, 0x008, 1);
            if !wait_set(RADIO, 0x10c) {
                return self.abort();
            }
            wr(RADIO, 0x110, 0);
            wr(RADIO, 0x010, 1);
            if wait_set(RADIO, 0x110) {
                true
            } else {
                self.stuck = true;
                false
            }
        }
    }
    unsafe fn abort(&mut self) -> bool {
        wr(RADIO, 0x110, 0);
        wr(RADIO, 0x010, 1);
        if !wait_set(RADIO, 0x110) {
            self.stuck = true;
        }
        false
    }
}

fn packet_set(index: usize, value: u8) {
    unsafe {
        (*PACKET.0.get())[index] = value;
    }
}

unsafe fn wr(base: usize, offset: usize, value: u32) {
    write_volatile((base + offset) as *mut u32, value)
}
unsafe fn rd(base: usize, offset: usize) -> u32 {
    read_volatile((base + offset) as *const u32)
}
unsafe fn wait_set(base: usize, offset: usize) -> bool {
    for _ in 0..WAIT_LIMIT {
        if rd(base, offset) != 0 {
            return true;
        }
    }
    false
}
