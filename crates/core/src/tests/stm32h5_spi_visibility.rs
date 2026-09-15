// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end SPI visibility gate for the STM32 H5-class "SPI v3" IP, on the
//! PRODUCTION path.
//!
//! # What this proves, and why it is built this way
//!
//! The claim under test is "a logic analyzer clipped to an H5-class SPI pad
//! measures the bus". That claim is only worth anything if the thing being
//! measured is the bus a REAL LAB builds, so every case here:
//!
//! * loads the COMMITTED chip yaml from `configs/chips/` off disk, and
//! * builds through [`SystemBus::from_config`] — the same call
//!   `system::node::build_cortex_m_node` makes for a real run — so the pad
//!   routing under test is the routing that ships.
//!
//! It deliberately does NOT hand-build a bus and call `wire_stm32_spi_pads`
//! itself. That shape proves the narrator works while proving nothing about
//! whether anything ships it, and it is exactly how the ESP32-S3 stayed dark in
//! production behind a green test.
//!
//! The decoder below is INDEPENDENT of the model: it replays the captured edge
//! stream, samples MOSI on the sampling edge CPOL/CPHA select, and reassembles
//! bytes MSB-first. It shares no code with [`SpiNarrator`] and knows nothing
//! about how the waveform was planned — if the narrator and the decoder agree,
//! the agreement is about the wire.
//!
//! [`SpiNarrator`]: crate::peripherals::spi_waveform::SpiNarrator

#[cfg(test)]
mod stm32h5_spi_visibility_tests {
    use crate::cpu::CortexM;
    use crate::logic_capture::LogicEdge;
    use crate::logic_capture::LogicSource;
    use crate::{Bus, Machine};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const RAM_BASE: u64 = 0x2000_0000;

    // ── SPI v3 register offsets (RM0481 §41; mirrored by this repo's
    // SVD-derived schema configs/peripherals/stm32h563/spi1.yaml) ────────────
    const CR1: u64 = 0x00;
    const CR2: u64 = 0x04;
    const CFG1: u64 = 0x08;
    const CFG2: u64 = 0x0C;
    const TXDR: u64 = 0x20;

    const CR1_SPE: u32 = 1 << 0;
    const CR1_CSTART: u32 = 1 << 9;
    const CR1_SSI: u32 = 1 << 12;
    const CFG2_MASTER: u32 = 1 << 22;
    const CFG2_SSM: u32 = 1 << 26;
    const CFG2_CPHA: u32 = 1 << 24;
    const CFG2_CPOL: u32 = 1 << 25;
    /// CFG1.MBR = 1 ⇒ a divide-by-4 SCK, well clear of the `bit_time >= 2`
    /// floor so half-periods stay distinguishable.
    const CFG1_MBR_DIV4: u32 = 1 << 28;

    /// One chip's SPI pad wiring, as the datasheet assigns it. The AF number is
    /// carried so a table drift in `bus::attach` shows up here as a pad that
    /// never routes rather than as a silently different pin.
    struct Case {
        /// `configs/chips/<chip>.yaml`.
        chip: &'static str,
        spi: &'static str,
        /// GPIO port peripheral id, pin, and AF nibble for SCK.
        sck: (&'static str, u8, u8),
        mosi: (&'static str, u8, u8),
        /// The datasheet this row was read from, quoted in failure messages so
        /// whoever sees a red gate knows which page to re-check.
        source: &'static str,
    }

    /// Every H5-class part whose SPI AF map was verified against the vendor
    /// datasheet. A part whose pinout could NOT be verified belongs nowhere in
    /// this list — an unverified pin table is the silent wrong-pad failure the
    /// F4/L4 I²C split exists to prevent.
    const CASES: &[Case] = &[
        Case {
            chip: "stm32h563",
            spi: "spi1",
            sck: ("gpioa", 5, 5),
            mosi: ("gpioa", 7, 5),
            source: "DS14258 Rev 6 Table 15, page 106",
        },
        Case {
            chip: "stm32h735",
            spi: "spi1",
            sck: ("gpioa", 5, 5),
            mosi: ("gpioa", 7, 5),
            source: "DS13312 Rev 4 Table 9, page 96",
        },
        Case {
            // ⚠️ The WBA52 is the reason `SpiPadMap` exists: PB4 is SPI1_SCK
            // here where the H563/H735 put SPI1_MISO, and its MOSI is on port A
            // while SCK sits on port B. Reading this row with the H5 table
            // would publish the clock onto the data pad.
            chip: "stm32wba52",
            spi: "spi1",
            sck: ("gpiob", 4, 5),
            mosi: ("gpioa", 15, 5),
            source: "DS14127 Rev 10 Table 25, pages 76-77",
        },
    ];

    /// Bytes clocked out. Chosen so a bit-order or edge-selection mistake cannot
    /// pass: `0x80` and `0x01` are the two single-bit patterns that swap under
    /// LSB-first, and `0xA5`/`0x5A` are each other's bit reversal.
    const WIRE_BYTES: [u8; 6] = [0xA5, 0x5A, 0x80, 0x01, 0xFF, 0x00];

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
            name: "stm32h5-spi-visibility".to_string(),
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

    /// Build the machine the production path builds: committed yaml →
    /// `from_config` → `configure_cortex_m` → `Machine`. A NOP slab in RAM
    /// gives `step()` something to execute so engine cycles advance
    /// deterministically.
    fn machine_for(case: &Case) -> Machine<CortexM> {
        let path = repo_root(&format!("configs/chips/{}.yaml", case.chip));
        let chip = ChipDescriptor::from_file(&path)
            .unwrap_or_else(|e| panic!("{}: load chip yaml: {e}", case.chip));
        let abs = path.to_string_lossy().to_string();
        let mut bus = crate::bus::SystemBus::from_config(&chip, &manifest_for(&abs))
            .unwrap_or_else(|e| panic!("{}: from_config: {e}", case.chip));
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        // `movs r0, #0` × 511 then a Thumb `b` back to the top.
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    fn base_of(machine: &Machine<CortexM>, name: &str) -> u64 {
        let idx = machine
            .bus
            .find_peripheral_index_by_name(name)
            .unwrap_or_else(|| panic!("peripheral '{name}' absent"));
        machine.bus.peripherals[idx].base
    }

    /// Ungate the SPI exactly as firmware does — through the RCC enable bit the
    /// chip yaml declares — rather than through the measurement-only clock
    /// bypass. The gate is READ OFF THE BUILT BUS (`clock_gate`), so this stays
    /// correct if a chip moves its enable bit.
    fn enable_spi_clock(machine: &mut Machine<CortexM>, case: &Case) {
        let idx = machine
            .bus
            .find_peripheral_index_by_name(case.spi)
            .unwrap_or_else(|| panic!("{}: no {}", case.chip, case.spi));
        let Some(gate) = machine.bus.peripherals[idx].clock_gate.clone() else {
            return; // ungated in this chip's yaml — nothing to enable
        };
        let rcc = base_of(machine, "rcc");
        // Satisfy EVERY bit the gate requires, not just the first: a peripheral
        // may need a bus-enable bit AND a kernel-clock ready bit.
        for req in &gate.requires {
            let cur = machine.bus.read_u32(rcc + req.reg_offset).unwrap();
            machine
                .bus
                .write_u32(rcc + req.reg_offset, cur | (1 << req.bit))
                .unwrap();
        }
    }

    /// Put one pad in alternate-function mode with the given AF nibble.
    fn route_pad(machine: &mut Machine<CortexM>, port: &str, pin: u8, af: u8) {
        let base = base_of(machine, port);
        // MODER (0x00): 0b10 = alternate function.
        let moder = machine.bus.read_u32(base).unwrap();
        let moder = (moder & !(0b11 << (pin * 2))) | (0b10 << (pin * 2));
        machine.bus.write_u32(base, moder).unwrap();
        // AFRL (0x20) for pins 0-7, AFRH (0x24) for 8-15.
        let (afr_off, shift) = if pin < 8 {
            (0x20u64, u32::from(pin) * 4)
        } else {
            (0x24u64, (u32::from(pin) - 8) * 4)
        };
        let afr = machine.bus.read_u32(base + afr_off).unwrap();
        let afr = (afr & !(0xF << shift)) | (u32::from(af) << shift);
        machine.bus.write_u32(base + afr_off, afr).unwrap();
    }

    /// Bring the SPI v3 controller up as a master and start a transfer of
    /// `frames` frames.
    ///
    /// Order matters and is the silicon's, not a convenience: `CFG1`/`CFG2` are
    /// write-protected while SPE=1, and a `CFG2.MASTER` request while the
    /// internal SS is low (SSM=1, CR1.SSI=0) mode-faults — so SSI is raised
    /// first.
    fn configure(machine: &mut Machine<CortexM>, spi: u64, cpol: bool, cpha: bool, frames: u32) {
        machine.bus.write_u32(spi + CR1, CR1_SSI).unwrap();
        let mut cfg2 = CFG2_MASTER | CFG2_SSM;
        if cpol {
            cfg2 |= CFG2_CPOL;
        }
        if cpha {
            cfg2 |= CFG2_CPHA;
        }
        machine.bus.write_u32(spi + CFG2, cfg2).unwrap();
        // DSIZE = 7 ⇒ 8-bit frames; MBR = 1 ⇒ SCK = kernel/4.
        machine
            .bus
            .write_u32(spi + CFG1, 7 | CFG1_MBR_DIV4)
            .unwrap();
        machine.bus.write_u32(spi + CR2, frames).unwrap();
        machine.bus.write_u32(spi + CR1, CR1_SSI | CR1_SPE).unwrap();
        machine
            .bus
            .write_u32(spi + CR1, CR1_SSI | CR1_SPE | CR1_CSTART)
            .unwrap();
    }

    /// Reconstruct a channel's level at `cycle` from its initial level and the
    /// recorded transitions.
    fn level_at(initial: bool, edges: &[LogicEdge], ch: u32, cycle: u64) -> bool {
        let mut level = initial;
        for e in edges {
            if e.ch == ch && e.cycle <= cycle {
                level = e.value;
            }
        }
        level
    }

    /// An INDEPENDENT SPI decoder over the captured edges.
    ///
    /// Knows only CPOL/CPHA and the two channel numbers. It finds the SCK
    /// transitions that are SAMPLING edges for the mode, reads MOSI as it stood
    /// at each, and packs bits MSB-first. It never consults the model.
    fn decode(edges: &[LogicEdge], ch_sck: u32, ch_mosi: u32, cpol: bool, cpha: bool) -> Vec<u8> {
        // A clock PULSE is a leading edge followed by a trailing one. Tracking
        // that pairing — rather than just filtering on edge direction — is what
        // makes this decoder immune to transitions that are not clock pulses at
        // all.
        //
        // There is exactly one such transition here and it is real silicon
        // behaviour: enabling the peripheral parks SCK at the programmed idle
        // polarity, so with CPOL=1 on a wire resting low the wire genuinely
        // rises BEFORE the first frame. That rise is in the "trailing"
        // direction, and a decoder that blindly sampled every trailing edge
        // would take it as a data bit and shift the entire stream by one.
        let mut sampling: Vec<u64> = Vec::new();
        let mut armed = false;
        for e in edges.iter().filter(|e| e.ch == ch_sck) {
            let leading = e.value != cpol;
            if leading {
                armed = true;
                if !cpha {
                    // CPHA=0 samples ON the leading edge.
                    sampling.push(e.cycle);
                }
            } else if armed {
                armed = false;
                if cpha {
                    // CPHA=1 samples on the trailing edge that CLOSES a pulse.
                    sampling.push(e.cycle);
                }
            }
        }
        sampling
            .chunks(8)
            .filter(|c| c.len() == 8)
            .map(|chunk| {
                chunk.iter().fold(0u8, |acc, &cycle| {
                    (acc << 1) | u8::from(level_at(false, edges, ch_mosi, cycle))
                })
            })
            .collect()
    }

    /// Clock `bytes` out and return the captured edges.
    fn run_transfer(case: &Case, cpol: bool, cpha: bool, bytes: &[u8]) -> (Vec<LogicEdge>, bool) {
        let mut machine = machine_for(case);
        enable_spi_clock(&mut machine, case);
        route_pad(&mut machine, case.sck.0, case.sck.1, case.sck.2);
        route_pad(&mut machine, case.mosi.0, case.mosi.1, case.mosi.2);

        let sck_idx = machine
            .bus
            .find_peripheral_index_by_name(case.sck.0)
            .expect("sck port");
        let mosi_idx = machine
            .bus
            .find_peripheral_index_by_name(case.mosi.0)
            .expect("mosi port");
        let initial = machine.logic_watch(&[
            Some(LogicSource::pad(sck_idx, case.sck.1)),
            Some(LogicSource::pad(mosi_idx, case.mosi.1)),
        ]);
        let idle_sck = initial[0].expect("SCK pad readable");

        let spi = base_of(&machine, case.spi);
        configure(&mut machine, spi, cpol, cpha, bytes.len() as u32);
        for &b in bytes {
            machine.bus.write_u32(spi + TXDR, u32::from(b)).unwrap();
        }
        // Let the narration burst reach the pads. The flush is paced against
        // the wire's own duration, so it lands once enough cycles have passed
        // for the frames to have really crossed.
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        (machine.logic_read_edges(0).edges, idle_sck)
    }

    // ── The gate ────────────────────────────────────────────────────────────

    /// THE headline property: on every verified H5-class part, bytes written to
    /// TXDR come back out of an independent decoder reading the PADS.
    #[test]
    fn h5_spi_bytes_are_recoverable_from_the_pads_on_every_verified_part() {
        for case in CASES {
            let (edges, _idle) = run_transfer(case, false, false, &WIRE_BYTES);
            assert!(
                !edges.is_empty(),
                "{}: {} clocked {} bytes and the pads stayed FLAT. The bus is \
                 invisible: a probe here reads the GPIO output latch, not the \
                 wire. ({})",
                case.chip,
                case.spi,
                WIRE_BYTES.len(),
                case.source,
            );
            let decoded = decode(&edges, 0, 1, false, false);
            assert_eq!(
                decoded,
                WIRE_BYTES.to_vec(),
                "{}: an independent mode-0 decoder reading the {} pads did not \
                 recover the bytes the controller shifted. ({})",
                case.chip,
                case.spi,
                case.source,
            );
        }
    }

    /// The waveform must be right in every CPOL/CPHA combination, not just the
    /// mode the default HAL happens to program. A narrator that ignored CPHA
    /// would still pass mode 0.
    #[test]
    fn h5_spi_decodes_in_every_clock_mode() {
        for case in CASES {
            for cpol in [false, true] {
                for cpha in [false, true] {
                    let (edges, _idle) = run_transfer(case, cpol, cpha, &WIRE_BYTES);
                    let decoded = decode(&edges, 0, 1, cpol, cpha);
                    assert_eq!(
                        decoded,
                        WIRE_BYTES.to_vec(),
                        "{}: mode {}{} lost the stream ({})",
                        case.chip,
                        u8::from(cpol),
                        u8::from(cpha),
                        case.source,
                    );
                }
            }
        }
    }

    /// SCK must idle at the programmed polarity and pulse once per bit. A
    /// waveform carrying the right bytes at the wrong clock count would decode
    /// only because the decoder counts the same wrong edges.
    #[test]
    fn h5_spi_clocks_exactly_one_pulse_per_bit() {
        for case in CASES {
            for cpol in [false, true] {
                let (edges, _) = run_transfer(case, cpol, false, &WIRE_BYTES);
                let sck: Vec<bool> = edges
                    .iter()
                    .filter(|e| e.ch == 0)
                    .map(|e| e.value)
                    .collect();
                let leading = sck.iter().filter(|&&v| v != cpol).count();
                assert_eq!(
                    leading,
                    WIRE_BYTES.len() * 8,
                    "{}: one leading SCK edge per clocked bit (cpol={cpol})",
                    case.chip,
                );
                // Edges must strictly alternate — a line cannot leave CPOL
                // twice without returning. This is what "every pulse closes"
                // means, stated so the ONE legitimate non-pulse transition
                // (SPE parking SCK at CPOL before the first frame, which with
                // CPOL=1 is a real rising edge) does not read as a miscount.
                for pair in sck.windows(2) {
                    assert_ne!(
                        pair[0], pair[1],
                        "{}: SCK reported the same level twice running \
                         (cpol={cpol}) — a pulse that never closed",
                        case.chip,
                    );
                }
                assert_eq!(
                    sck.last().copied(),
                    Some(cpol),
                    "{}: SCK must come to rest at its programmed CPOL idle \
                     (cpol={cpol})",
                    case.chip,
                );
            }
        }
    }

    /// MOSI must never move ON a sampling edge — that is the setup/hold
    /// property a real receiver depends on, and a trace that violates it is not
    /// a waveform any silicon could produce.
    #[test]
    fn h5_spi_mosi_is_stable_across_every_sampling_edge() {
        for case in CASES {
            for cpha in [false, true] {
                let (edges, _) = run_transfer(case, false, cpha, &WIRE_BYTES);
                let sampling: Vec<u64> = edges
                    .iter()
                    .filter(|e| e.ch == 0 && if cpha { !e.value } else { e.value })
                    .map(|e| e.cycle)
                    .collect();
                for e in edges.iter().filter(|e| e.ch == 1) {
                    assert!(
                        !sampling.contains(&e.cycle),
                        "{}: MOSI moved on a sampling edge at cycle {} (cpha={cpha})",
                        case.chip,
                        e.cycle,
                    );
                }
            }
        }
    }

    /// An LSB-first transfer must publish NOTHING rather than a plausible trace
    /// that decodes to the bit-reversed byte.
    ///
    /// This is the one failure mode that cannot be caught downstream: a
    /// reversed byte looks like a valid waveform, so a user reads confident
    /// garbage. A gap is honest; a reversed word is not.
    #[test]
    fn h5_spi_refuses_to_narrate_lsb_first_rather_than_reversing_the_bytes() {
        const CFG2_LSBFRST: u32 = 1 << 23;
        for case in CASES {
            let mut machine = machine_for(case);
            enable_spi_clock(&mut machine, case);
            route_pad(&mut machine, case.sck.0, case.sck.1, case.sck.2);
            route_pad(&mut machine, case.mosi.0, case.mosi.1, case.mosi.2);
            let sck_idx = machine
                .bus
                .find_peripheral_index_by_name(case.sck.0)
                .expect("sck port");
            let mosi_idx = machine
                .bus
                .find_peripheral_index_by_name(case.mosi.0)
                .expect("mosi port");
            machine.logic_watch(&[
                Some(LogicSource::pad(sck_idx, case.sck.1)),
                Some(LogicSource::pad(mosi_idx, case.mosi.1)),
            ]);
            let spi = base_of(&machine, case.spi);
            machine.bus.write_u32(spi + CR1, CR1_SSI).unwrap();
            machine
                .bus
                .write_u32(spi + CFG2, CFG2_MASTER | CFG2_SSM | CFG2_LSBFRST)
                .unwrap();
            machine
                .bus
                .write_u32(spi + CFG1, 7 | CFG1_MBR_DIV4)
                .unwrap();
            machine
                .bus
                .write_u32(spi + CR2, WIRE_BYTES.len() as u32)
                .unwrap();
            machine.bus.write_u32(spi + CR1, CR1_SSI | CR1_SPE).unwrap();
            machine
                .bus
                .write_u32(spi + CR1, CR1_SSI | CR1_SPE | CR1_CSTART)
                .unwrap();
            for &b in &WIRE_BYTES {
                machine.bus.write_u32(spi + TXDR, u32::from(b)).unwrap();
            }
            for _ in 0..20_000 {
                machine.step().unwrap();
            }
            let edges = machine.logic_read_edges(0).edges;
            assert!(
                edges.is_empty(),
                "{}: an LSB-first transfer published {} edges. The narrator \
                 draws MSB-first only, so this trace decodes to the \
                 bit-reversed byte — a plausible-looking lie. It must publish \
                 nothing instead.",
                case.chip,
                edges.len(),
            );
        }
    }

    /// Enabling the peripheral must park SCK at CPOL BEFORE any data moves.
    ///
    /// Real silicon drives SCK to the programmed idle polarity the moment SPE
    /// hands the pad to the SPI — not when the first byte is written. Getting
    /// this wrong is subtle rather than loud: the parking transition simply
    /// migrates INTO the first narrated frame, where (with CPOL=1 on a wire
    /// resting low) it is an extra rising edge sitting among the frame's own.
    /// A decoder that pairs leading edges with trailing ones survives that, so
    /// no byte-level assertion here notices — which is exactly why this needs
    /// its own test rather than being assumed covered.
    #[test]
    fn enabling_the_peripheral_parks_sck_at_cpol_before_any_frame() {
        for case in CASES {
            for cpol in [false, true] {
                let mut machine = machine_for(case);
                enable_spi_clock(&mut machine, case);
                route_pad(&mut machine, case.sck.0, case.sck.1, case.sck.2);
                route_pad(&mut machine, case.mosi.0, case.mosi.1, case.mosi.2);
                let spi = base_of(&machine, case.spi);
                // Bring the controller up, but write NOTHING to TXDR.
                configure(&mut machine, spi, cpol, false, WIRE_BYTES.len() as u32);
                let sck_idx = machine
                    .bus
                    .find_peripheral_index_by_name(case.sck.0)
                    .expect("sck port");
                let level = machine.bus.peripherals[sck_idx]
                    .dev
                    .read_gpio_pad(case.sck.1)
                    .expect("the SCK pad is AF-routed and reads the wire");
                assert_eq!(
                    level, cpol,
                    "{}: with SPE set and CPOL={cpol}, the SCK pad must already \
                     rest at the programmed idle BEFORE the first frame — no \
                     TXDR write has happened yet. ({})",
                    case.chip, case.source,
                );
            }
        }
    }

    /// An H5-class SPI whose chip yaml declares NO `pad_map` must route
    /// nowhere.
    ///
    /// This is the fail-closed half of [`SpiPadMap`], and it cannot be observed
    /// on the three shipping parts because all three DECLARE a map. So the case
    /// is constructed: take the real H563 descriptor, strip the declaration,
    /// and require the SPI pads to go dark rather than fall back to some
    /// default table. Guessing between two tables that disagree about which pin
    /// is SCK and which is MISO is worse than an honest gap on the
    /// bus-visibility board.
    #[test]
    fn an_h5_spi_with_no_declared_pad_map_routes_nothing() {
        let path = repo_root("configs/chips/stm32h563.yaml");
        let text = std::fs::read_to_string(&path).expect("read h563 yaml");
        assert!(
            text.contains("pad_map:"),
            "fixture drift: the H563 yaml no longer declares a pad_map, so \
             stripping it proves nothing",
        );
        let stripped: String = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("pad_map:"))
            .collect::<Vec<_>>()
            .join("\n");
        let chip: ChipDescriptor =
            serde_yaml::from_str(&stripped).expect("parse the stripped descriptor");
        let abs = path.to_string_lossy().to_string();
        let bus = crate::bus::SystemBus::from_config(&chip, &manifest_for(&abs))
            .expect("build the stripped bus");
        let spi_pads: Vec<&str> = bus
            .bound_pad_functions()
            .into_iter()
            .filter(|f| f.starts_with("SPI"))
            .collect();
        assert!(
            spi_pads.is_empty(),
            "an H5 SPI with no declared pad_map bound {spi_pads:?}. It must \
             route NOTHING: the H5 register file is shared by parts whose \
             pinouts disagree (H563/H735 put SPI1_SCK on PB3 where the WBA52 \
             puts SPI1_MISO), so falling back to a default table publishes one \
             part's pinout onto another's silicon.",
        );
        // The other buses must be untouched — this is a SPI-specific gate, not
        // a bus that stopped building.
        assert!(
            bus.bound_pad_functions()
                .iter()
                .any(|f| f.starts_with("I2C")),
            "stripping pad_map must not disturb I²C pad routing",
        );
    }

    /// The two predicates must stay APART.
    ///
    /// `is_stm32_wire_layout` answers "has a bit engine"; `publishes_stm32_pad_wire`
    /// answers "can put a waveform on a pad". The H5 is the part where the two
    /// answers differ, and conflating them is a live hazard: the unmerged
    /// `feat/spi-edge-sampling` branch uses the FORMER to refuse edge-accurate
    /// slave sampling to "STM32H5 SPIv3 / Kinetis DSPI". If a later change
    /// widens that predicate to make pads route, this test fails and says why —
    /// before the merge silently grants edge sampling to a byte-level engine.
    #[test]
    fn pad_publication_and_bit_engine_are_separate_questions() {
        use crate::peripherals::spi::{Spi, SpiRegisterLayout};

        let h5 = Spi::new_with_layout(SpiRegisterLayout::Stm32H5);
        assert!(
            h5.publishes_stm32_pad_wire(),
            "the H5 narrates a waveform onto pads",
        );
        assert!(
            !h5.is_stm32_wire_layout(),
            "the H5 has NO bit engine: `write_stm32h5_reg` moves a whole frame \
             inside the TXDR write (ctsize -= 1, EOT at zero) with no tick \
             countdown and no bit index. Widening `is_stm32_wire_layout` to \
             include it would make pads route AND would defeat \
             feat/spi-edge-sampling's refusal to sample it edge-accurately. \
             Add a pad-routing predicate instead.",
        );

        let classic = Spi::new_with_layout(SpiRegisterLayout::Stm32);
        assert!(
            classic.is_stm32_wire_layout(),
            "classic drives a bit engine"
        );
        assert!(
            classic.publishes_stm32_pad_wire(),
            "classic publishes pads too — both answers are true here, which is \
             why the H5 is the case that keeps them honest",
        );
    }
}
