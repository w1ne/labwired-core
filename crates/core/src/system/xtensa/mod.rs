// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 / ESP32-S3 system glue.
//!
//! `configure_xtensa_esp32s3` registers all peripherals defined for the
//! ESP32-S3-Zero and returns a fresh `XtensaLx7` CPU.  After calling this,
//! the caller invokes `boot::esp32s3::fast_boot` to load an ELF and
//! synthesise CPU state, then enters the simulation loop.

use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;

mod esp32;
pub use esp32::*;
mod esp32s3;
pub use esp32s3::*;

/// Phase B compatibility shim. Delegates to `configure_xtensa_esp32s3`
/// with default options and discards the wiring (icache/dcache backings)
/// that callers from Phase B's CLI/Python paths don't yet consume.
pub fn configure_xtensa(bus: &mut SystemBus) -> XtensaLx7 {
    configure_xtensa_esp32s3(bus, &Esp32s3Opts::default()).cpu
}

/// Flat-array `Peripheral` used for IRAM + DRAM + flash XIP mappings.
///
/// Pub so `SystemBus::fetch_slice` can downcast and hand the CPU a raw
/// pointer into the backing buffer for the IRAM/flash fetch-cache fast
/// path (#119 Phase 1.2). The `data` field stays private; access is via
/// [`backing_ptr_len`] which returns a raw `(*const u8, usize)` pair.
///
/// INVARIANT: `data` is allocated once in [`new`] and never re-sized
/// (no `push`, `extend`, `resize`, `clear`). All read/write paths index
/// in-place via slice access, and `restore_runtime_snapshot` requires
/// the new bytes match the existing length. This stability is what makes
/// it safe to hand a raw `*const u8` to the CPU and re-use it across
/// many `step()` calls.
pub struct RamPeripheral {
    data: std::cell::RefCell<Vec<u8>>,
}

impl RamPeripheral {
    pub fn new(size: usize) -> Self {
        Self {
            data: std::cell::RefCell::new(vec![0u8; size]),
        }
    }

    /// Allocate `size` bytes and preload the low bytes from `image` (used to
    /// load a real ROM dump). The buffer stays exactly `size` (image is
    /// truncated/zero-padded), preserving the fixed-length INVARIANT.
    pub fn with_image(size: usize, image: &[u8]) -> Self {
        let mut buf = vec![0u8; size];
        let n = image.len().min(size);
        buf[..n].copy_from_slice(&image[..n]);
        Self {
            data: std::cell::RefCell::new(buf),
        }
    }

    /// Return a raw pointer + length to the backing buffer for the
    /// fetch-cache fast path. The pointer is stable for the lifetime
    /// of `self` because [`Self::new`] is the only allocation site and
    /// no path resizes the `Vec` (see struct-level INVARIANT).
    ///
    /// Reading through this pointer is safe iff no concurrent
    /// `borrow_mut()` is live AND `self` is not moved/dropped while
    /// the pointer is in use. The fetch-cache holds the pointer only
    /// across read-only `step()` calls; any bus write that lands in
    /// the cached range MUST invalidate the cache first so we never
    /// race a fetch against a `borrow_mut()`.
    pub fn backing_ptr_len(&self) -> (*const u8, usize) {
        let d = self.data.borrow();
        (d.as_ptr(), d.len())
    }
}

impl std::fmt::Debug for RamPeripheral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RamPeripheral({}B)", self.data.borrow().len())
    }
}

impl crate::Peripheral for RamPeripheral {
    fn read(&self, offset: u64) -> crate::SimResult<u8> {
        Ok(*self.data.borrow().get(offset as usize).unwrap_or(&0))
    }
    fn write(&mut self, offset: u64, value: u8) -> crate::SimResult<()> {
        let mut d = self.data.borrow_mut();
        if let Some(slot) = d.get_mut(offset as usize) {
            *slot = value;
        }
        Ok(())
    }

    // Word/halfword bulk paths. The default trait impl decomposes each
    // multi-byte access into N byte calls — for Xtensa every instruction
    // fetch (read_u32 of IRAM) hits this 4 times per instruction, each
    // taking a fresh RefCell::borrow() + Vec bounds check. The single-shot
    // slice path here cuts each fetch from 4 borrows to 1.
    // See labwired-core#119 (JIT roadmap Phase 1.1).
    fn read_u16(&self, offset: u64) -> crate::SimResult<u16> {
        let d = self.data.borrow();
        let off = offset as usize;
        let bytes = d.get(off..off + 2);
        Ok(match bytes {
            Some(b) => u16::from_le_bytes([b[0], b[1]]),
            None => 0, // out-of-range reads return 0 to match the byte path
        })
    }
    fn read_u32(&self, offset: u64) -> crate::SimResult<u32> {
        let d = self.data.borrow();
        let off = offset as usize;
        let bytes = d.get(off..off + 4);
        Ok(match bytes {
            Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            None => 0,
        })
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> crate::SimResult<()> {
        let mut d = self.data.borrow_mut();
        let off = offset as usize;
        if let Some(slot) = d.get_mut(off..off + 2) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> crate::SimResult<()> {
        let mut d = self.data.borrow_mut();
        let off = offset as usize;
        if let Some(slot) = d.get_mut(off..off + 4) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }

    /// Dump the backing buffer verbatim. Snapshot stays compact (a 200 KiB
    /// DRAM round-trips as 200 KiB on disk — bincode adds an 8-byte length
    /// prefix and that's it).
    fn runtime_snapshot(&self) -> Vec<u8> {
        self.data.borrow().clone()
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        let mut d = self.data.borrow_mut();
        if bytes.len() != d.len() {
            return Err(crate::SimulationError::NotImplemented(format!(
                "RamPeripheral runtime snapshot size mismatch: expected {} bytes, got {}",
                d.len(),
                bytes.len()
            )));
        }
        d.copy_from_slice(bytes);
        Ok(())
    }

    /// Expose `&dyn Any` so `SystemBus::fetch_slice` can downcast and
    /// reach `backing_ptr_len` without a virtual-call detour.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;
    use crate::Peripheral;
    use std::sync::{Arc, Mutex};

    /// The esp32 (LX6) factory + ESP32_PERIPHERALS table must build each
    /// peripheral with the same window (base, size) as the hand-wired
    /// `configure_xtensa_esp32`. That builder also registers memory regions and
    /// catch-all stubs, so this checks the table's peripherals by name rather
    /// than comparing whole buses. Pins the factory path as equivalent before it
    /// replaces the hand-wired registrations.
    #[test]
    fn esp32_factory_descriptors_match_hardwired() {
        use labwired_config::PeripheralConfig;
        use std::collections::HashMap;

        let mut hw = SystemBus::new();
        let _ = configure_xtensa_esp32(&mut hw);

        for &(id, ty, base, size, irq) in ESP32_PERIPHERALS {
            let cfg = PeripheralConfig {
                id: id.to_string(),
                r#type: ty.to_string(),
                base_address: base,
                size: None,
                irq,
                clock: None,
                config: HashMap::new(),
            };
            assert!(
                crate::peripherals::esp32::factory::try_build(ty, &cfg).is_some(),
                "esp32 factory missing type {ty} for {id}"
            );
            let idx = hw
                .find_peripheral_index_by_name(id)
                .unwrap_or_else(|| panic!("hand-wired esp32 bus missing {id}"));
            let p = &hw.peripherals[idx];
            assert_eq!((p.base, p.size), (base, size), "window mismatch for {id}");
        }
    }

    /// The esp32s3 factory + canonical descriptor table must place exactly the
    /// same peripheral windows (name, base, size) as the hand-wired
    /// `register_esp32s3_peripherals`. This pins the Stage-3 data-driven path as
    /// equivalent before it replaces the hand-wired one. (i2c0's TMP102 slave is
    /// internal model state, not a window, so it does not affect this check.)
    #[test]
    fn factory_descriptors_match_hardwired_peripherals() {
        use labwired_config::PeripheralConfig;
        use std::collections::HashMap;

        let mut hw = SystemBus::new();
        hw.peripherals.clear();
        register_esp32s3_peripherals(&mut hw, &Esp32s3Opts::default());

        let mut fac = SystemBus::new();
        fac.peripherals.clear();
        for &(id, ty, base, size, irq) in ESP32S3_PERIPHERALS {
            let cfg = PeripheralConfig {
                id: id.to_string(),
                r#type: ty.to_string(),
                base_address: base,
                size: None,
                irq,
                clock: None,
                config: HashMap::new(),
            };
            let dev = crate::peripherals::esp32s3::factory::try_build(ty, &cfg)
                .unwrap_or_else(|| panic!("esp32s3 factory missing type {ty}"));
            // Bus-entry irq is None on both paths; the source id is baked into
            // the model by the factory via cfg.irq.
            fac.add_peripheral(id, base, size, None, dev);
        }

        let windows = |b: &SystemBus| {
            let mut v: Vec<(String, u64, u64, Option<u32>)> = b
                .peripherals
                .iter()
                .map(|p| (p.name.clone(), p.base, p.size, p.irq))
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            windows(&hw),
            windows(&fac),
            "factory/table path must place the same peripheral windows as the hand-wired path"
        );
    }

    #[test]
    fn configure_registers_all_peripherals() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        // Confirm core regions are reachable.
        assert!(bus.read_u8(0x4037_0000).is_ok(), "IRAM");
        assert!(bus.read_u8(0x3FC8_8000).is_ok(), "DRAM");
        assert!(bus.read_u8(0x4200_0000).is_ok(), "flash I-cache");
        assert!(bus.read_u8(0x3C00_0000).is_ok(), "flash D-cache");
        assert!(bus.read_u8(0x6003_8000).is_ok(), "USB_SERIAL_JTAG");
        assert!(bus.read_u8(0x6002_3000).is_ok(), "SYSTIMER");
        assert!(bus.read_u8(0x600C_0000).is_ok(), "SYSTEM");
        assert!(bus.read_u8(0x6000_8000).is_ok(), "RTC_CNTL");
        assert!(bus.read_u8(0x6000_7000).is_ok(), "EFUSE");
    }

    /// Empirical routing proof for the layered 0x600C_0000 SYSTEM region.
    /// Verifies WHICH peripheral the bus router dispatches each probe offset to
    /// (by the distinct window each owns) and that behavior is correct:
    ///   * crosscore_ipi (size 0x10) still serves 0x030..0x03C,
    ///   * the faithful SYSTEM model (windows A/B) serves the register block
    ///     and `resolve_window(0x600C_0000)` (window A, NON-round-tripping),
    ///   * the big SystemStub (size 0x1_0000) still serves ≥ 0x600C_1000.
    #[test]
    fn system_register_block_routing() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        let ipi_base = crate::peripherals::esp32s3::crosscore_ipi::BASE; // 0x600C_0030
        let ipi_size = crate::peripherals::esp32s3::crosscore_ipi::SIZE; // 0x10

        // 1. resolve_window(0x600C_0000) must return the faithful model's
        //    window A (base 0x600C_0000, size 0x30) — NON-round-tripping, so
        //    the probe builds a clean baseline and credits the modeled
        //    registers. (The model is split into two windows straddling the
        //    crosscore_ipi hole; window A is what the probe resolves.)
        let (base, size) = bus.resolve_window(0x600C_0000).expect("SYSTEM window");
        assert_eq!(base, 0x600C_0000);
        assert_eq!(size, ipi_base - 0x600C_0000, "window A size = 0x30");

        // 2. crosscore_ipi (size 0x10) still serves 0x030..0x03C — the boot
        //    doorbell must NOT be shadowed by the SYSTEM model. The hole in the
        //    SYSTEM windows guarantees this regardless of the hint cache, so
        //    re-query after touching a neighbouring SYSTEM register to prove
        //    the cache does not pull it back into a SYSTEM window.
        let _ = bus.read_u32(0x600C_0008); // pollute hint with window A
        for off in [0x030u64, 0x034, 0x038, 0x03C] {
            let (b, s) = bus.resolve_window(0x600C_0000 + off).expect("ipi window");
            assert_eq!(
                (b, s),
                (ipi_base, ipi_size),
                "offset {off:#x} must route to crosscore_ipi (hole preserved)"
            );
        }

        // 3a. Window A serves an architected register (PERIP_CLK_EN0 @ 0x018):
        //     reads its HW reset value and round-trips a masked write.
        assert_eq!(
            bus.read_u32(0x600C_0018).unwrap(),
            0xF9C1_E06F,
            "PERIP_CLK_EN0 reads its HW-validated reset value"
        );
        bus.write_u32(0x600C_0018, 0x1234_5678).unwrap();
        assert_eq!(
            bus.read_u32(0x600C_0018).unwrap(),
            0x1234_5678,
            "PERIP_CLK_EN0 round-trips a write under its mask"
        );

        // 3b. Window B serves the high registers (RTC_FASTMEM_CONFIG @ 0x050,
        //     DATE @ 0xFFC) with correct absolute-offset translation.
        assert_eq!(
            bus.read_u32(0x600C_0050).unwrap(),
            0x7FF0_0000,
            "RTC_FASTMEM_CONFIG reset value (window B, base-offset translated)"
        );
        assert_eq!(
            bus.read_u32(0x600C_0FFC).unwrap(),
            0x0210_1220,
            "DATE constant (window B tail)"
        );

        // 3c. An unmapped offset inside window B reads as zero and does NOT
        //     round-trip (the anti-gaming property the coverage probe relies on).
        assert_eq!(bus.read_u32(0x600C_0100).unwrap(), 0, "unmapped reads 0");
        bus.write_u32(0x600C_0100, 0xFFFF_FFFF).unwrap();
        assert_eq!(
            bus.read_u32(0x600C_0100).unwrap(),
            0,
            "unmapped offset must NOT round-trip"
        );

        // 4. The big SystemStub (size 0x1_0000) still serves the region above
        //    the register block, e.g. 0x600C_1800 (interrupt-map / accelerator).
        let (b, s) = bus.resolve_window(0x600C_1800).expect("stub window");
        assert_eq!(
            (b, s),
            (0x600C_0000, 0x1_0000),
            "0x600C_1800 → big SystemStub"
        );
        bus.write_u32(0x600C_1800, 0xDEAD_BEEF).unwrap();
        assert_eq!(bus.read_u32(0x600C_1800).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn iram_writeable_and_readable() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        bus.write_u8(0x4037_0010, 0xAB).unwrap();
        assert_eq!(bus.read_u8(0x4037_0010).unwrap(), 0xAB);
    }

    /// Phase 1.2: `fetch_slice` MUST hand back a `&[u8]` that aliases
    /// the RAM peripheral's backing store, observe writes that go
    /// through the bus, and only cover RAM-backed regions.
    #[test]
    fn fetch_slice_aliases_iram_and_skips_non_ram() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        // IRAM range 0x4037_0000 + 0x40000. fetch_slice should cover
        // the whole region (~256 KiB) and observe a fresh byte write.
        let pc = 0x4037_0010u64;
        bus.write_u8(pc, 0x37).unwrap();
        let (start, end, slice) = bus.fetch_slice(pc).expect("IRAM fetch_slice");
        assert!(start <= pc && pc < end, "pc must lie in returned range");
        let off = (pc - start) as usize;
        assert_eq!(slice[off], 0x37, "slice must mirror current RAM byte");

        // Writes propagate without invalidating the slice itself —
        // the consumer is responsible for invalidating its cached
        // pointer when a write lands in-range. We just need to see
        // the new value through the same slice (vec is in place).
        bus.write_u8(pc, 0xA5).unwrap();
        let (start, _, slice) = bus.fetch_slice(pc).unwrap();
        assert_eq!(slice[(pc - start) as usize], 0xA5);

        // GPIO at 0x6000_4000 is not a RamPeripheral — slow path
        // must stay active there.
        assert!(
            bus.fetch_slice(0x6000_4000).is_none(),
            "GPIO must not serve fetch_slice"
        );
    }

    #[test]
    fn configure_registers_gpio_io_mux_intmatrix() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        let names: Vec<&str> = bus.peripherals.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"gpio"), "gpio missing; have: {names:?}");
        assert!(names.contains(&"io_mux"), "io_mux missing; have: {names:?}");
        assert!(
            names.contains(&"intmatrix"),
            "intmatrix missing; have: {names:?}"
        );
    }

    #[test]
    fn add_gpio_observer_installs_on_gpio_peripheral() {
        use crate::peripherals::esp32s3::gpio::GpioObserver;

        #[derive(Debug, Default)]
        struct CountObserver(Mutex<u32>);
        impl GpioObserver for CountObserver {
            fn on_pin_change(&self, _pin: u8, _from: bool, _to: bool, _sim_cycle: u64) {
                *self.0.lock().unwrap() += 1;
            }
        }

        let mut bus = SystemBus::new();
        let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        let obs = Arc::new(CountObserver::default());
        wiring.add_gpio_observer(&mut bus, obs.clone());

        // Trigger a GPIO transition by writing OUT_W1TS bit 5 via the bus.
        // GPIO base 0x6000_4000, OUT_W1TS at offset 0x08.
        bus.write_u8(0x6000_4008, 0x20).unwrap(); // bit 5 = 0x20
        bus.write_u8(0x6000_4009, 0).unwrap();
        bus.write_u8(0x6000_400A, 0).unwrap();
        bus.write_u8(0x6000_400B, 0).unwrap();

        assert!(*obs.0.lock().unwrap() >= 1, "observer should have fired");
    }

    #[test]
    fn configure_registers_i2c0_with_tmp102_attached() {
        let mut bus = SystemBus::new();
        let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        // I2C0 should be present at 0x6001_3000.
        let names: Vec<_> = bus.peripherals.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"i2c0"), "i2c0 missing; have: {names:?}");

        // The attached TMP102 should respond at address 0x48 by setting
        // INT_NACK to 0 after a one-byte write probe.
        let i2c_idx = bus
            .peripherals
            .iter()
            .position(|p| p.name == "i2c0")
            .unwrap();
        let i2c_any = bus.peripherals[i2c_idx]
            .dev
            .as_any_mut()
            .expect("i2c0 should expose as_any_mut");
        let i2c = i2c_any
            .downcast_mut::<crate::peripherals::esp32s3::i2c::Esp32s3I2c>()
            .expect("downcast to Esp32s3I2c");

        // Build a probe: RSTART; WRITE 1 (addr+W=0x90); STOP.
        // Opcodes per ESP32-S3 TRM § 29.5: 1=WRITE, 2=STOP, 6=RSTART.
        i2c.write_u32(0x58, 6u32 << 11).unwrap(); // RSTART (opcode 6)
        i2c.write_u32(0x5C, (1u32 << 11) | 1).unwrap(); // WRITE 1 byte
        i2c.write_u32(0x60, 2u32 << 11).unwrap(); // STOP (opcode 2)
        i2c.write_u32(0x1C, 0x90).unwrap(); // addr+W (DATA at 0x1c)
        i2c.write_u32(0x04, 1 << 5).unwrap(); // TRANS_START
        let int_raw = i2c.read_u32(0x20).unwrap();
        assert_eq!(
            int_raw & (1 << 11),
            0,
            "TMP102 attached at 0x48 must ACK; got INT_RAW=0x{int_raw:08x}"
        );
    }

    #[test]
    fn flash_xip_windows_have_independent_backings() {
        // Real silicon shares the SPI flash between both windows but each has
        // its own MMU page table; for fast-boot we model this as two distinct
        // backing buffers so that ELFs with .rodata at 0x3c000020 and .text at
        // 0x42000020 don't collide on the same physical offset. Force fast-boot
        // so the assertion is deterministic regardless of whether the host has
        // the ESP toolchain ROM installed (which would auto-select faithful mode).
        std::env::set_var("LABWIRED_ESP32S3_FASTBOOT", "1");
        let mut bus = SystemBus::new();
        let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        std::env::remove_var("LABWIRED_ESP32S3_FASTBOOT");
        wiring.icache_backing.lock().unwrap()[0] = 0xCA;
        wiring.dcache_backing.lock().unwrap()[0] = 0xFE;
        assert_eq!(bus.read_u8(0x4200_0000).unwrap(), 0xCA, "I-cache alias");
        assert_eq!(bus.read_u8(0x3C00_0000).unwrap(), 0xFE, "D-cache alias");
    }

    /// `configure_xtensa_esp32` + `attach_esp32_external_devices` must register
    /// `spi3` on the bus and attach an `Ssd1680Tricolor290` panel to it when the
    /// manifest declares an `ssd1680_tricolor_290` external device on `spi3`.
    ///
    /// This is the unit-level guard that the manifest/CLI path (which was
    /// previously broken — `config_error: external device 'epaper' references
    /// missing connection 'spi3'`) now wires up correctly.
    #[test]
    fn attach_esp32_external_devices_registers_spi3_and_epaper() {
        use labwired_config::{ExternalDevice, SystemManifest};
        use std::collections::HashMap;

        // Build a minimal manifest that declares the SSD1680 e-paper panel
        // on spi3 — matching the real `configs/systems/esp32-wroom-epaper.yaml`.
        let mut config = HashMap::new();
        config.insert(
            "cs_pin".to_string(),
            serde_yaml::Value::String("GPIO5".to_string()),
        );
        let manifest = SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "test-esp32-epaper".to_string(),
            chip: "esp32.yaml".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            peripherals: vec![],
            external_devices: vec![ExternalDevice {
                id: "epaper".to_string(),
                r#type: "ssd1680_tricolor_290".to_string(),
                connection: "spi3".to_string(),
                route: Default::default(),
                config,
            }],
            board_io: vec![],
            debug_uart: None,
        };

        let mut bus = SystemBus::new();
        let _cpu = configure_xtensa_esp32(&mut bus);

        // spi3 must exist after configure_xtensa_esp32.
        assert!(
            bus.find_peripheral_index_by_name("spi3").is_some(),
            "spi3 must be registered by configure_xtensa_esp32"
        );

        // Attaching external devices must succeed (no 'missing connection' error).
        attach_esp32_external_devices(&mut bus, &manifest)
            .expect("attach_esp32_external_devices must not error for spi3 + epaper");

        // The SSD1680 panel must now be attached to spi3.
        let idx = bus
            .find_peripheral_index_by_name("spi3")
            .expect("spi3 still present after attach");
        let any = bus.peripherals[idx]
            .dev
            .as_any()
            .expect("spi3 supports as_any");
        let spi = any
            .downcast_ref::<crate::peripherals::esp32::spi::Esp32Spi>()
            .expect("spi3 is Esp32Spi");
        let panel_count = spi
            .attached_devices
            .iter()
            .filter(|d| {
                d.as_any()
                    .and_then(|a| {
                        a.downcast_ref::<crate::peripherals::components::Ssd1680Tricolor290>()
                    })
                    .is_some()
            })
            .count();
        assert_eq!(
            panel_count, 1,
            "exactly one Ssd1680Tricolor290 should be attached to spi3"
        );
    }

    /// S3 config wiring: `attach_esp32_external_devices` must also handle the
    /// ESP32-S3 GP-SPI model (`Esp32s3Spi`) — an S3 manifest wiring a device
    /// to `spi3_s3` previously errored with "not an ESP32 SPI peripheral"
    /// because only the classic `Esp32Spi` downcast was attempted.
    #[test]
    fn attach_esp32_external_devices_attaches_to_s3_gpspi() {
        use labwired_config::{ExternalDevice, SystemManifest};
        use std::collections::HashMap;

        let mut config = HashMap::new();
        config.insert(
            "cs_pin".to_string(),
            serde_yaml::Value::String("GPIO10".to_string()),
        );
        let manifest = SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "test-esp32s3-epaper".to_string(),
            chip: "esp32s3.yaml".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            peripherals: vec![],
            external_devices: vec![ExternalDevice {
                id: "epaper".to_string(),
                r#type: "ssd1680_tricolor_290".to_string(),
                connection: "spi3_s3".to_string(),
                route: Default::default(),
                config,
            }],
            board_io: vec![],
            debug_uart: None,
        };

        // Register spi3_s3 exactly as the production S3 bring-up does
        // (configure_xtensa_esp32s3: Esp32s3Spi::new(22) @ 0x6002_5000),
        // without the full heavyweight S3 bus construction.
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "spi3_s3",
            0x6002_5000,
            0x100,
            None,
            Box::new(crate::peripherals::esp32s3::gpspi::Esp32s3Spi::new(22)),
        );

        attach_esp32_external_devices(&mut bus, &manifest)
            .expect("attach must succeed for an Esp32s3Spi connection");

        let idx = bus.find_peripheral_index_by_name("spi3_s3").unwrap();
        let spi = bus.peripherals[idx]
            .dev
            .as_any()
            .unwrap()
            .downcast_ref::<crate::peripherals::esp32s3::gpspi::Esp32s3Spi>()
            .expect("spi3_s3 is Esp32s3Spi");
        assert_eq!(
            spi.attached_device_count(),
            1,
            "exactly one panel attached to the S3 GP-SPI controller"
        );
    }

    /// `attach_esp32_external_devices` must return an error (not panic) when
    /// the manifest references a peripheral name that doesn't exist on the bus.
    #[test]
    fn attach_esp32_external_devices_errors_on_missing_connection() {
        use labwired_config::{ExternalDevice, SystemManifest};

        let manifest = SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "test".to_string(),
            chip: "esp32.yaml".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            peripherals: vec![],
            external_devices: vec![ExternalDevice {
                id: "epaper".to_string(),
                r#type: "ssd1680_tricolor_290".to_string(),
                connection: "spi99".to_string(), // does not exist
                route: Default::default(),
                config: std::collections::HashMap::new(),
            }],
            board_io: vec![],
            debug_uart: None,
        };

        let mut bus = SystemBus::new();
        let _cpu = configure_xtensa_esp32(&mut bus);

        let result = attach_esp32_external_devices(&mut bus, &manifest);
        assert!(
            result.is_err(),
            "should error when connection peripheral is missing"
        );
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("spi99"),
            "error message should name the missing peripheral; got: {msg}"
        );
    }
}
