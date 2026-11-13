//! Cortex-M7 L1 cache: the cache-maintenance operations in the System Control
//! Space and the geometry the SCB's cache identification registers report.
//!
//! The Cortex-M7 has optional instruction and data caches whose sizes the SoC
//! integrator picks (0, 4, 8, 16, 32 or 64 KiB each). Software finds them
//! through the SCB identification registers (CLIDR 0xE000_ED78, CTR 0xED7C,
//! CCSIDR 0xED80 selected by CSSELR 0xED84) and maintains them through the
//! write-only operations at 0xE000_EF50..0xE000_EF78 (ARMv7-M ARM DDI 0403E,
//! the cache maintenance operations; Cortex-M7 TRM DDI 0489, the L1 cache
//! chapter).
//!
//! A chip that has the caches declares this model at `0xE000_EF50`:
//!
//! ```yaml
//! - id: "l1_cache"
//!   type: "cortex_m7_cache"
//!   base_address: 0xE000EF50
//!   size: 48
//!   config: { icache_kb: 16, dcache_kb: 16 }
//! ```
//!
//! `configure_cortex_m` finds it through `Peripheral::l1_cache_geometry` and
//! gives its [`L1CacheGeometry`] to the SCB,
//! so CMSIS `SCB_EnableDCache`/`SCB_InvalidateDCache` see the real number of
//! sets and ways. A chip without it keeps the old behaviour: the identification
//! registers read zero and the maintenance window is unmapped.
//!
//! What is not modelled: cache contents. Simulated memory is always coherent,
//! so every maintenance operation (invalidate, clean, clean-and-invalidate, by
//! address or by set/way, and the branch-predictor invalidate) has no
//! architectural effect here and is accepted as a no-op. A firmware that forgets
//! to clean a DMA buffer, or reads stale data after a missing invalidate, works
//! in the simulator and fails on silicon: that class of bug is not caught.
//! CCR.IC/CCR.DC are stored by the SCB and gate nothing.

use crate::{Peripheral, SimResult};

/// ICIALLU .. BPIALL, offsets from 0xE000_EF50 (DDI 0403E, the SCS cache
/// maintenance register table).
const ICIALLU: u64 = 0x00;
const ICIMVAU: u64 = 0x08;
const DCIMVAC: u64 = 0x0C;
const DCISW: u64 = 0x10;
const DCCMVAU: u64 = 0x14;
const DCCMVAC: u64 = 0x18;
const DCCSW: u64 = 0x1C;
const DCCIMVAC: u64 = 0x20;
const DCCISW: u64 = 0x24;
const BPIALL: u64 = 0x28;

/// The I- and D-cache sizes in KiB (0 = absent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct L1CacheGeometry {
    pub icache_kb: u32,
    pub dcache_kb: u32,
}

impl L1CacheGeometry {
    /// The sizes a Cortex-M7 can be built with (TRM DDI 0489).
    pub const SIZES_KB: [u32; 6] = [0, 4, 8, 16, 32, 64];

    pub fn new(icache_kb: u32, dcache_kb: u32) -> anyhow::Result<Self> {
        for (name, kb) in [("icache_kb", icache_kb), ("dcache_kb", dcache_kb)] {
            if !Self::SIZES_KB.contains(&kb) {
                anyhow::bail!(
                    "cortex_m7_cache: {name} = {kb} is not a Cortex-M7 cache size (0/4/8/16/32/64)"
                );
            }
        }
        Ok(Self {
            icache_kb,
            dcache_kb,
        })
    }

    /// CLIDR: LoUU = 1, LoC = 1 and Ctype1 = the caches present at level 1
    /// (0x0900_0003 with both caches, TRM DDI 0489).
    pub fn clidr(&self) -> u32 {
        let ctype1 = u32::from(self.icache_kb != 0) | (u32::from(self.dcache_kb != 0) << 1);
        if ctype1 == 0 {
            return 0;
        }
        (1 << 27) | (1 << 24) | ctype1
    }

    /// CTR: the fixed Cortex-M7 value, 32-byte lines on both sides
    /// (0x8303_C003, TRM DDI 0489).
    pub fn ctr(&self) -> u32 {
        0x8303_C003
    }

    /// CCSIDR for the cache CSSELR selects: bit 0 = InD (1 = instruction).
    /// Both caches have 32-byte lines (LineSize = 1); the data cache is
    /// 4-way, the instruction cache 2-way; WT/WB/RA/WA all read 1. These give
    /// the TRM's CCSIDR encodings (TRM DDI 0489), e.g. a 16 KiB D-cache reads
    /// 0xF00F_E019 and a 16 KiB I-cache 0xF01F_E009.
    pub fn ccsidr(&self, csselr: u32) -> u32 {
        let (kb, ways) = if csselr & 1 != 0 {
            (self.icache_kb, 2)
        } else {
            (self.dcache_kb, 4)
        };
        // Level other than 1, or an absent cache: no such cache.
        if csselr & 0xE != 0 || kb == 0 {
            return 0;
        }
        let sets = kb * 1024 / (ways * 32);
        (0xF << 28) | ((sets - 1) << 13) | ((ways - 1) << 3) | 1
    }
}

/// The maintenance-operation window at 0xE000_EF50 (48 bytes).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CortexM7Cache {
    geometry: L1CacheGeometry,
    /// Maintenance operations seen since reset, for inspection; they do
    /// nothing else (see the module docs).
    #[serde(default)]
    ops: u64,
}

impl CortexM7Cache {
    pub fn new(geometry: L1CacheGeometry) -> Self {
        Self { geometry, ops: 0 }
    }

    pub fn geometry(&self) -> L1CacheGeometry {
        self.geometry
    }

    pub fn maintenance_ops(&self) -> u64 {
        self.ops
    }
}

impl Peripheral for CortexM7Cache {
    // A pure register window with no `tick()`.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    /// The operations are write-only; a read is UNKNOWN in the architecture
    /// and returns zero here.
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        // A byte store only counts once per word.
        if offset & 3 == 0 {
            self.write_u32(offset, 0)?;
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, _value: u32) -> SimResult<()> {
        match offset & !3 {
            ICIALLU | ICIMVAU | DCIMVAC | DCISW | DCCMVAU | DCCMVAC | DCCSW | DCCIMVAC | DCCISW
            | BPIALL => self.ops += 1,
            _ => crate::census_reg!("cortex_m7_cache:CortexM7Cache", offset & !3, "write"),
        }
        Ok(())
    }

    fn on_node_reset(&mut self) {
        self.ops = 0;
    }

    fn l1_cache_geometry(&self) -> Option<L1CacheGeometry> {
        Some(self.geometry)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "CortexM7Cache snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_matches_the_cortex_m7_trm_table() {
        let g = L1CacheGeometry::new(16, 16).unwrap();
        assert_eq!(g.clidr(), 0x0900_0003);
        assert_eq!(g.ctr(), 0x8303_C003);
        assert_eq!(g.ccsidr(0), 0xF00F_E019, "16 KiB D-cache");
        assert_eq!(g.ccsidr(1), 0xF01F_E009, "16 KiB I-cache");
        let g = L1CacheGeometry::new(32, 4).unwrap();
        assert_eq!(g.ccsidr(0), 0xF003_E019, "4 KiB D-cache");
        assert_eq!(g.ccsidr(1), 0xF03F_E009, "32 KiB I-cache");
        // Level 2 does not exist.
        assert_eq!(g.ccsidr(2), 0);
    }

    #[test]
    fn absent_caches_read_as_absent() {
        let g = L1CacheGeometry::new(0, 16).unwrap();
        assert_eq!(g.clidr(), 0x0900_0002);
        assert_eq!(g.ccsidr(1), 0);
        assert_eq!(L1CacheGeometry::new(0, 0).unwrap().clidr(), 0);
        assert!(L1CacheGeometry::new(12, 16).is_err());
    }

    #[test]
    fn maintenance_ops_are_accepted_and_counted() {
        let mut c = CortexM7Cache::new(L1CacheGeometry::new(16, 16).unwrap());
        c.write_u32(ICIALLU, 0).unwrap();
        c.write_u32(DCISW, 0x4000_0FE0).unwrap();
        c.write_u32(DCCIMVAC, 0x2040_0000).unwrap();
        assert_eq!(c.maintenance_ops(), 3);
        assert_eq!(c.read(DCISW).unwrap(), 0);
        c.on_node_reset();
        assert_eq!(c.maintenance_ops(), 0);
    }
}
