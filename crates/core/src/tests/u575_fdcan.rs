// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32U575 FDCAN1 gate — the chip declaration the Arduino L8_can sketch
//! drives over MMIO.
//!
//! The matrix sketch (`validation/arduino-matrix/sketches/L8_can`) takes the
//! Bosch M_CAN through the register-level loopback sequence on the
//! NUCLEO-U575ZI-Q. This file builds the COMMITTED `configs/chips/stm32u575.yaml`
//! through the production [`SystemBus::from_config`] path — the same build the
//! CLI/matrix use — and pins what the sketch depends on:
//!
//! * `fdcan1` at the SVD base `0x4000_A400` with a 4 KiB window that covers the
//!   shared message RAM (`FDCAN1_RAM` @ `0x4000_AC00`, offset `0x800`), NVIC
//!   line 39 (`FDCAN1_IT0`);
//! * the enable gate is `RCC_APB1ENR2.FDCAN1EN` (bit 9, offset `0xA0`) — the
//!   U5 puts FDCAN1 in APB1ENR2, not APB1ENR1, and at the same 0xA0 offset the
//!   H5 uses for APB1HENR, which is exactly the kind of drift the SVD names;
//! * writes are swallowed until firmware opens that gate;
//! * the exact L8 sequence loops a standard frame back into RX FIFO0.
//!
//! The U575 has exactly ONE FDCAN instance: DS13737 says "1 CAN FD controller",
//! and both the vendored SVD and ST's `stm32u575xx.h` declare only `FDCAN1`.
//! There is no `fdcan2` @ `0x4000_A800` on this part, so none is declared.

#[cfg(test)]
mod u575_fdcan_tests {
    use crate::bus::SystemBus;
    use crate::tests::machine_advance::CountingCpu;
    use crate::{AdvanceRequest, Bus, Machine};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const FDCAN_BASE: u64 = 0x4000_A400;
    const FDCAN_RAM: u64 = 0x800;
    const REG_CREL: u64 = 0x000;
    const REG_ENDN: u64 = 0x004;
    const REG_TEST: u64 = 0x010;
    const REG_CCCR: u64 = 0x018;
    const REG_RXF0S: u64 = 0x090;
    const REG_TXBAR: u64 = 0x0CC;
    const REG_TXFQS: u64 = 0x0C4;

    const RCC_BASE: u64 = 0x4602_0C00;
    const RCC_APB1ENR2: u64 = 0x0A0;
    const FDCAN1EN_BIT: u8 = 9;

    const TEST_LBCK: u32 = 1 << 4;

    fn repo_root(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("repo root")
            .join(rel)
    }

    fn manifest_for(chip_path: &str) -> SystemManifest {
        SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "u575-fdcan".to_string(),
            chip: chip_path.to_string(),
            cpu_hz: None,
            external_devices: vec![],
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            board_io: vec![],
            debug_uart: None,
            wifi_ap: None,
            peripherals: vec![],
            memory_overrides: Default::default(),
        }
    }

    /// The production build: committed chip yaml → `SystemBus::from_config`.
    fn u575_bus() -> SystemBus {
        let path = repo_root("configs/chips/stm32u575.yaml");
        let descriptor =
            ChipDescriptor::from_file(&path).expect("load committed stm32u575 chip yaml");
        let abs = path.to_string_lossy().into_owned();
        SystemBus::from_config(&descriptor, &manifest_for(&abs)).expect("build stm32u575 bus")
    }

    fn open_fdcan1_gate(bus: &mut SystemBus) {
        let cur = bus.read_u32(RCC_BASE + RCC_APB1ENR2).unwrap();
        bus.write_u32(RCC_BASE + RCC_APB1ENR2, cur | (1 << FDCAN1EN_BIT))
            .unwrap();
    }

    #[test]
    fn fdcan1_is_declared_at_the_u5_svd_base_behind_apb1enr2() {
        let bus = u575_bus();
        let idx = bus
            .find_peripheral_index_by_name("fdcan1")
            .expect("fdcan1 must be declared in configs/chips/stm32u575.yaml");
        let p = &bus.peripherals[idx];
        assert_eq!(p.base, FDCAN_BASE, "SVD FDCAN1 base");
        assert_eq!(p.size, 0x1000, "4 KiB window covers SRAMCAN @ +0x800");
        assert_eq!(
            p.irq,
            Some(39),
            "SVD FDCAN1_IT0 (IT1 = 40, ILS not modeled)"
        );

        let gate = p.clock_gate.as_ref().expect("FDCAN1 is clock-gated on U5");
        assert_eq!(gate.requires.len(), 1, "one enable bit on U5");
        assert_eq!(
            gate.requires[0].reg_offset, RCC_APB1ENR2,
            "FDCAN1EN is in APB1ENR2 on U5 (RM0456/SVD) — not APB1ENR1"
        );
        assert_eq!(gate.requires[0].bit, FDCAN1EN_BIT);
    }

    #[test]
    fn fdcan1_writes_drop_until_apb1enr2_fdcan1en_is_set() {
        let mut bus = u575_bus();
        // Reset: the gate is clear, so the bus must swallow the write and the
        // controller stays in INIT.
        bus.write_u32(FDCAN_BASE + REG_CCCR, 0).unwrap();
        open_fdcan1_gate(&mut bus);
        assert_eq!(
            bus.read_u32(FDCAN_BASE + REG_CCCR).unwrap() & 1,
            1,
            "unclocked FDCAN1 writes must not leave INIT"
        );
        // Gate open: the same write lands.
        bus.write_u32(FDCAN_BASE + REG_CCCR, 0).unwrap();
        assert_eq!(bus.read_u32(FDCAN_BASE + REG_CCCR).unwrap() & 1, 0);
    }

    #[test]
    fn fdcan1_reset_values_match_the_u5_svd() {
        let mut bus = u575_bus();
        open_fdcan1_gate(&mut bus);
        assert_eq!(bus.read_u32(FDCAN_BASE + REG_CREL).unwrap(), 0x3214_1218);
        assert_eq!(bus.read_u32(FDCAN_BASE + REG_ENDN).unwrap(), 0x8765_4321);
        assert_eq!(bus.read_u32(FDCAN_BASE + REG_CCCR).unwrap(), 0x0000_0001);
        assert_eq!(bus.read_u32(FDCAN_BASE + REG_TXFQS).unwrap(), 0x0000_0003);
    }

    #[test]
    fn u5_fdcan1_loops_back_the_l8_can_sequence() {
        let mut bus = u575_bus();
        open_fdcan1_gate(&mut bus);

        // Byte-for-byte the L8_can sketch sequence (FD loopback branch).
        bus.write_u32(FDCAN_BASE + REG_CCCR, 0x3).unwrap(); // INIT | CCE
        bus.write_u32(FDCAN_BASE + REG_CCCR, 0xA3).unwrap(); // + TEST | MON
        bus.write_u32(FDCAN_BASE + REG_TEST, TEST_LBCK).unwrap();
        bus.write_u32(FDCAN_BASE + FDCAN_RAM + 0x278, 0x123 << 18)
            .unwrap();
        bus.write_u32(FDCAN_BASE + FDCAN_RAM + 0x27C, 1 << 16)
            .unwrap();
        bus.write_u32(FDCAN_BASE + FDCAN_RAM + 0x280, 0xA5).unwrap();
        bus.write_u32(FDCAN_BASE + REG_CCCR, 0xA2).unwrap(); // leave INIT
        bus.write_u32(FDCAN_BASE + REG_TXBAR, 1).unwrap();

        // The sketch spins on RXF0S. Completion must ride the production
        // machine lifecycle, not a raw bus tick: feature-off builds finish TX
        // on the next peripheral walk, while `event-scheduler` builds defer it
        // to the FDCAN event chain that only `Machine::advance` drains (the
        // walk skips scheduler-driven peripherals). Bounded rounds with a
        // generous fuel budget — completion is polled, never a magic count.
        let mut machine = Machine::new(CountingCpu::default(), bus);
        let mut fill = 0;
        for _ in 0..8 {
            machine
                .advance(AdvanceRequest::run(Some(64)))
                .expect("advance the U575 machine");
            fill = machine.bus.read_u32(FDCAN_BASE + REG_RXF0S).unwrap() & 0x7F;
            if fill != 0 {
                break;
            }
        }
        assert_eq!(fill, 1, "loopback frame lands in RX FIFO0");

        let r0 = machine.bus.read_u32(FDCAN_BASE + FDCAN_RAM + 0xB0).unwrap();
        let r2 = machine.bus.read_u32(FDCAN_BASE + FDCAN_RAM + 0xB8).unwrap();
        assert_eq!((r0 >> 18) & 0x7FF, 0x123, "standard ID survives loopback");
        assert_eq!(r2 & 0xFF, 0xA5, "payload byte survives loopback");
    }

    fn read_u32(machine: &dyn crate::world::MachineTrait, addr: u64) -> u32 {
        let bytes = std::array::from_fn(|byte| machine.read_u8(addr + byte as u64).unwrap());
        u32::from_le_bytes(bytes)
    }

    fn write_u32(machine: &mut dyn crate::world::MachineTrait, addr: u64, value: u32) {
        for (byte, value) in value.to_le_bytes().into_iter().enumerate() {
            machine.write_u8(addr + byte as u64, value).unwrap();
        }
    }

    /// Two U575 nodes over the manifest-wired `can_bus`, the same shape as the
    /// H563 `world_can_bus` fixtures but through the U5 RCC gate
    /// (`APB1ENR2.FDCAN1EN`) and the production `World::from_manifest` path.
    #[test]
    fn u5_fdcan1_reaches_a_peer_through_the_world_can_bus() {
        use crate::world::World;
        use labwired_config::{EnvironmentManifest, InterconnectConfig, NodeConfig};
        use std::collections::HashMap;

        fn node(id: &str) -> NodeConfig {
            NodeConfig {
                id: id.to_string(),
                system: "crates/core/tests/fixtures/u575-can-world-system.yaml".to_string(),
                firmware: "tests/fixtures/stm32u575-arduino-serial.elf".to_string(),
                config_overrides: HashMap::new(),
                profile: None,
            }
        }

        let environment = EnvironmentManifest {
            schema_version: "1.0".to_string(),
            name: "two-u5-can-nodes".to_string(),
            nodes: vec![node("tester"), node("ecu")],
            interconnects: vec![InterconnectConfig {
                r#type: "can_bus".to_string(),
                nodes: vec!["tester".to_string(), "ecu".to_string()],
                config: HashMap::from([(
                    "peripheral".to_string(),
                    serde_yaml::Value::String("fdcan1".to_string()),
                )]),
            }],
            rf: None,
        };

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("repo root")
            .to_path_buf();
        let mut world =
            World::from_manifest(environment, &root).expect("a two-node U5 FDCAN bus should build");

        for id in ["tester", "ecu"] {
            let machine = world.machines.get_mut(id).unwrap().as_mut();
            // The U5 gate is APB1ENR2 bit 9; no frame moves before firmware
            // opens it. Take the controller out of INIT afterwards.
            let cur = read_u32(machine, RCC_BASE + RCC_APB1ENR2);
            write_u32(machine, RCC_BASE + RCC_APB1ENR2, cur | (1 << FDCAN1EN_BIT));
            write_u32(machine, FDCAN_BASE + REG_CCCR, 0);
        }

        let tester = world.machines.get_mut("tester").unwrap().as_mut();
        write_u32(tester, FDCAN_BASE + FDCAN_RAM + 0x278, 0x321 << 18);
        write_u32(tester, FDCAN_BASE + FDCAN_RAM + 0x27C, 1 << 16);
        write_u32(tester, FDCAN_BASE + FDCAN_RAM + 0x280, 0xA5);
        write_u32(tester, FDCAN_BASE + REG_TXBAR, 1);

        for _ in 0..2 {
            let results = world.step_all();
            assert!(
                results.values().all(Result::is_ok),
                "world round failed: {results:?}"
            );
        }

        let sender = world.machines.get("tester").unwrap();
        assert_eq!(
            read_u32(sender.as_ref(), FDCAN_BASE + REG_RXF0S) & 0x7F,
            0,
            "the transmitting node must not receive its own frame"
        );
        let receiver = world.machines.get("ecu").unwrap();
        assert_eq!(
            read_u32(receiver.as_ref(), FDCAN_BASE + REG_RXF0S) & 0x7F,
            1,
            "the peer receives exactly one frame"
        );
        let r0 = read_u32(receiver.as_ref(), FDCAN_BASE + FDCAN_RAM + 0xB0);
        let r2 = read_u32(receiver.as_ref(), FDCAN_BASE + FDCAN_RAM + 0xB8);
        assert_eq!((r0 >> 18) & 0x7FF, 0x321, "standard ID crosses the bus");
        assert_eq!(r2 & 0xFF, 0xA5, "payload byte crosses the bus");
    }
}
