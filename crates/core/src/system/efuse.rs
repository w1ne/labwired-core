// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Factory eFuse MAC — the ONE place a simulated ESP32 die gets its identity.
//!
//! Every ESP32 Espressif ships carries a 48-bit base MAC burned into eFuse
//! block 0 at manufacture, and it is unique to that die. It is not decoration:
//! it is the root of every address the chip presents to the world. `esp_read_mac`
//! derives the WiFi station MAC from it, and the Bluetooth controller derives
//! its BLE device address from it — which is why a sketch can write
//!
//! ```c
//! myTag = BLEDevice::getAddress().getNative()[5];   // "who am I"
//! ...
//! if ((uint8_t)m[2] == myTag) return;               // "that advert is mine"
//! ```
//!
//! and expect it to mean something. Two dice with one address break that
//! promise silently: every peer advertisement looks like an echo of the node's
//! own, and connectionless BLE — the whole point of which is *broadcast* —
//! deadlocks with both nodes waiting for a peer they can already hear.
//!
//! ## Faithfully modelled
//!
//! * **Distinctness.** Every die minted in a process gets an address no other
//!   die in that process has. That is the physical claim that matters, and it
//!   is the one the twin can actually keep.
//! * **Locally administered.** The first octet is `0x02` — bit 1 set, bit 0
//!   clear: a unicast, *locally administered* address. That is the honest way
//!   to say "this is a simulated die, not an Espressif-assigned identity".
//!   Handing out a plausible-looking Espressif OUI would be inventing
//!   provenance the twin does not have.
//!
//! ## Idealised — present, but not physical
//!
//! * **The address is not stable across process lifetimes.** On silicon the MAC
//!   is burned once and is the same on every power-up forever. Here it is
//!   allocated when the die is built, so a second Run in the same browser tab
//!   mints the *next* address rather than the same one. Distinctness holds;
//!   permanence does not. Pin [`RomBootOpts::pinned_efuse_mac`] to model one
//!   specific die across runs.
//!   [`RomBootOpts::pinned_efuse_mac`]: crate::boot::esp32c3_rom::RomBootOpts::pinned_efuse_mac
//! * **Only the base MAC is modelled**, not the rest of eFuse block 0 (CRC
//!   bytes, custom MAC block, the `MAC_FACTORY_CRC` the ROM would check).
//!
//! Not modelled at all: eFuse programming (a burn is permanent on silicon and
//! there is no burn path here), the custom-MAC block, and MAC address types
//! other than the base — `esp_read_mac`'s per-interface derivation is the
//! firmware's own arithmetic over this value, and it runs for real.

use std::sync::atomic::{AtomicU32, Ordering};

/// The first factory MAC an allocator hands out: `02:00:00:00:00:02`.
///
/// This exact value is load-bearing history, not an arbitrary starting point.
/// It is the station MAC every WiFi lab was seeded with before dies had
/// identities of their own (a *zero* eFuse MAC associates but does not stay
/// associated — a hard-won C3 WiFi rule), and it is node A's address in the
/// CLI's two-node BLE and WiFi runs. Keeping it first keeps every existing
/// single-MCU lab byte-identical.
pub const FIRST_FACTORY_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x02];

/// Hands out distinct factory MACs. One allocator = one "fab": every die it
/// mints differs from every other die it has minted.
///
/// The process-global one is [`next_factory_efuse_mac`]. Build a local
/// allocator to reason about the sequence in a test without racing whatever
/// else in the process is minting dice.
#[derive(Debug)]
pub struct FactoryMacAllocator {
    /// Low 16 bits of the next address, i.e. `mac[4] << 8 | mac[5]`.
    next: AtomicU32,
}

impl Default for FactoryMacAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl FactoryMacAllocator {
    pub const fn new() -> Self {
        Self {
            next: AtomicU32::new((FIRST_FACTORY_MAC[4] as u32) << 8 | FIRST_FACTORY_MAC[5] as u32),
        }
    }

    /// Mint the next die identity. Wraps after 65534 dice in one process, which
    /// is 65534 more MCUs than a lab has; the wrap keeps the address valid
    /// rather than pretending the allocator is infinite.
    pub fn next_mac(&self) -> [u8; 6] {
        let n = self.next.fetch_add(1, Ordering::Relaxed) & 0xFFFF;
        let mut mac = FIRST_FACTORY_MAC;
        mac[4] = (n >> 8) as u8;
        mac[5] = n as u8;
        mac
    }
}

/// The process-wide fab. Every die built without a pinned address takes its
/// identity from here, so two MCUs in one lab — which is two `Machine`s in one
/// process, whether that is the browser tab, the CLI, or a `World` — can never
/// come out sharing an address.
pub fn next_factory_efuse_mac() -> [u8; 6] {
    static FAB: FactoryMacAllocator = FactoryMacAllocator::new();
    FAB.next_mac()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole module exists for.
    #[test]
    fn every_die_gets_its_own_address() {
        let fab = FactoryMacAllocator::new();
        let macs: Vec<[u8; 6]> = (0..64).map(|_| fab.next_mac()).collect();
        for (i, a) in macs.iter().enumerate() {
            for b in &macs[i + 1..] {
                assert_ne!(a, b, "two dice from one fab share an address");
            }
        }
    }

    /// Byte-identity with what single-MCU labs were seeded with before.
    #[test]
    fn the_first_die_keeps_the_historical_station_mac() {
        let fab = FactoryMacAllocator::new();
        assert_eq!(fab.next_mac(), [0x02, 0x00, 0x00, 0x00, 0x00, 0x02]);
        // And node B of the CLI's two-node runs keeps its documented address.
        assert_eq!(fab.next_mac(), [0x02, 0x00, 0x00, 0x00, 0x00, 0x03]);
    }

    /// Unicast + locally administered: the honest way to say "simulated die".
    #[test]
    fn addresses_are_unicast_and_locally_administered() {
        let fab = FactoryMacAllocator::new();
        for _ in 0..1000 {
            let mac = fab.next_mac();
            assert_eq!(mac[0] & 0x01, 0, "multicast bit must be clear");
            assert_eq!(mac[0] & 0x02, 0x02, "locally-administered bit must be set");
        }
    }

    /// The global fab is the same fab for everyone.
    #[test]
    fn the_process_fab_never_repeats() {
        let a = next_factory_efuse_mac();
        let b = next_factory_efuse_mac();
        assert_ne!(a, b);
        assert_eq!(a[0] & 0x02, 0x02);
    }
}
