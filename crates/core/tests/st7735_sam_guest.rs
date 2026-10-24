// SPDX-License-Identifier: MIT
//! Authored blocking-SPI guest and controller-memory pixel proof. NOT native
//! Arcade, DMA, IRQ, actual module glass or BGR qualification.
use labwired_config::{ChipDescriptor, DeviceDescriptor, SystemManifest};
use labwired_core::{
    bus::{DevicePins, SystemBus},
    cpu::cortex_m::CortexM,
    inspect::{artifact_format, artifact_region_ink, Artifact, InspectOpts, PixelRegion},
    memory::ProgramImage,
    peripherals::{
        components::declarative_display::{DeclarativeDisplayKit, GenericDisplay, GlassWindow},
        kit::{AttachCtx, PeripheralKit},
        spi::SpiDevice,
    },
    Arch, Bus, Machine,
};

const SPI: u64 = 0x43000000;
const PORT: u64 = 0x41008080;
const PORT_A: u64 = 0x41008000;
const MCLK: u64 = 0x40000820;
const GCLK: u64 = 0x40001c00 + 0x80 + 34 * 4;
const MASTER: u32 = (3 << 2) | (2 << 16);
const MARKER: u32 = 0x51c0ffee;

#[derive(Clone, Copy, Debug)]
enum Wiring {
    Correct,
    MissingMclk,
    MissingGclk,
    WrongMux,
    CsHigh,
    DcHigh,
}

fn board(legacy: bool) -> SystemBus {
    board_with_controls(legacy, false)
}

fn board_with_controls(legacy: bool, controls: bool) -> SystemBus {
    board_with_window(
        legacy,
        controls,
        GlassWindow {
            col_offset: 0,
            row_offset: 0,
            cols: 2,
            rows: 2,
        },
    )
}

fn board_with_window(legacy: bool, controls: bool, window: GlassWindow) -> SystemBus {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    let manifest = SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    if legacy {
        assert!(bus
            .peripherals
            .iter_mut()
            .find(|p| p.name == "sercom4")
            .unwrap()
            .dev
            .force_legacy_walk());
        bus.recompute_walk_deletable();
        bus.refresh_peripheral_index();
    }
    let descriptor =
        DeviceDescriptor::from_yaml(include_str!("fixtures/st7735r-memory.yaml")).unwrap();
    let mut display = GenericDisplay::from_descriptor(&descriptor).unwrap();
    display.set_cs_pin("PB7");
    display.set_dc_pin("PB5");
    display.set_dc_source(PORT + 0x10, 5);
    // Fixture-only crop of real controller RAM. NOT an asserted module crop.
    display.set_glass_window(window);
    if controls {
        // Exercise the actual generic kit/AttachCtx path. Only enable its
        // fixture crop; the unregistered prototype remains unshipped.
        let yaml = include_str!("fixtures/st7735r-memory.yaml")
            .replace("    width: 132", "    glass_crop: true\n    width: 132");
        let kit = DeclarativeDisplayKit::from_yaml(&yaml).unwrap();
        let config = format!(
            "id: panel\ntype: st7735r-memory-prototype\nconnection: sercom4\nconfig:\n  cs_pin: PB7\n  dc_pin: PB5\n  reset_pin: PA0\n  backlight_pin: PA1\n  col_offset: {}\n  row_offset: {}\n  cols: {}\n  rows: {}\n",
            window.col_offset, window.row_offset, window.cols, window.rows,
        );
        let ext: labwired_config::ExternalDevice = serde_yaml::from_str(&config).unwrap();
        let before = bus.gpio_devices.len();
        kit.attach(&mut AttachCtx::new(&mut bus, &ext)).unwrap();
        assert_eq!(bus.gpio_devices.len(), before + 1);
        assert!(!bus.gpio_devices.last().unwrap().needs_per_cycle_service());
    } else {
        bus.attach_spi_device("sercom4", Box::new(display)).unwrap();
    }
    bus
}

/// Emit short aligned Thumb blocks with their own literal islands, so a full
/// RGBSET upload cannot exceed the 16-bit LDR's 1 KiB literal reach. Nothing
/// executes on the host; these bytes are the guest's actual instructions.
#[derive(Default)]
struct Guest {
    code: Vec<u8>,
}
impl Guest {
    fn halfwords(&mut self, ops: &[u16]) {
        for op in ops {
            self.code.extend(op.to_le_bytes());
        }
    }

    fn store(&mut self, addr: u64, value: u32, byte: bool) {
        assert_eq!(self.code.len() & 3, 0);
        // LDR r0,[PC,#4]; LDR r1,[PC,#8]; STR[B] r1,[r0]; B past literals.
        self.halfwords(&[0x4801, 0x4902, if byte { 0x7001 } else { 0x6001 }, 0xe003]);
        self.code.extend((addr as u32).to_le_bytes());
        self.code.extend(value.to_le_bytes());
    }

    fn txc(&mut self) {
        // LDR r0,[PC,#8]; LDRB r1,[r0]; MOVS r2,#2; TST; BEQ LDRB;
        // B past the INTFLAG address literal. TXC is guest-polled, not seeded.
        self.halfwords(&[0x4802, 0x7801, 0x2202, 0x4211, 0xd0fb, 0xe001]);
        self.code.extend(((SPI + 0x18) as u32).to_le_bytes());
    }

    fn send(&mut self, byte: u8, data: bool, wiring: Wiring) {
        let data = data || matches!(wiring, Wiring::DcHigh);
        self.store(PORT + if data { 0x18 } else { 0x14 }, 1 << 5, false);
        if !matches!(wiring, Wiring::CsHigh) {
            self.store(PORT + 0x14, 1 << 7, false);
        }
        self.store(SPI + 0x28, u32::from(byte), true);
        self.txc();
        self.store(PORT + 0x18, 1 << 7, false);
    }

    fn command(&mut self, op: u8, args: &[u8], wiring: Wiring) {
        self.send(op, false, wiring);
        for &byte in args {
            self.send(byte, true, wiring);
        }
    }

    fn image(mut self) -> ProgramImage {
        self.halfwords(&[0xe7fe]); // park, never fall into a data island
        let mut bytes = vec![0u8; 0x100];
        bytes[0..4].copy_from_slice(&0x20004000u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&0x4101u32.to_le_bytes());
        bytes.extend(self.code);
        let mut image = ProgramImage::new(0x4101, Arch::Arm);
        image.add_segment(0x4000, bytes);
        image
    }
}

fn guest(wiring: Wiring) -> ProgramImage {
    guest_with_controls(wiring, None)
}

#[derive(Clone, Copy, Debug)]
enum Controls {
    Released,
    BacklightOff,
    HeldReset,
    UndrivenReset,
    MuxedReset,
    WrongPort,
    ResetWithSpiClockOff,
}

fn guest_with_controls(wiring: Wiring, controls: Option<Controls>) -> ProgramImage {
    guest_with_stream(wiring, controls, None)
}

fn guest_with_stream(
    wiring: Wiring,
    controls: Option<Controls>,
    rectangular_axes_swapped: Option<bool>,
) -> ProgramImage {
    guest_with_driver_trace(wiring, controls, rectangular_axes_swapped, None, false)
}

fn guest_with_driver_trace(
    wiring: Wiring,
    controls: Option<Controls>,
    rectangular_axes_swapped: Option<bool>,
    commands: Option<&serde_json::Value>,
    truncate_lut: bool,
) -> ProgramImage {
    let mut guest = guest_trace_prefix(
        wiring,
        controls,
        rectangular_axes_swapped,
        commands,
        truncate_lut,
    );
    guest.store(0x20000000, MARKER, false);
    guest.image()
}

// Return the actual instruction stream before its completion marker so a
// staged guest can continue with protocol writes, never host pixel injection.
fn guest_trace_prefix(
    wiring: Wiring,
    controls: Option<Controls>,
    rectangular_axes_swapped: Option<bool>,
    commands: Option<&serde_json::Value>,
    truncate_lut: bool,
) -> Guest {
    let mut guest = Guest::default();
    if let Some(controls) = controls {
        let level = match controls {
            Controls::HeldReset => 2,
            Controls::BacklightOff => 1,
            _ => 3,
        };
        guest.store(PORT_A + 0x18, level, false);
        guest.store(
            PORT_A,
            if matches!(controls, Controls::UndrivenReset | Controls::WrongPort) {
                2
            } else {
                3
            },
            false,
        );
        if matches!(controls, Controls::WrongPort) {
            // PB0 does not release PA0, even though the bit number is equal.
            guest.store(PORT + 0x18, 1, false);
            guest.store(PORT, 1, false);
        }
        if matches!(controls, Controls::MuxedReset) {
            guest.store(PORT_A + 0x40, 1, true);
        }
    }
    for (addr, value, byte) in [
        (
            MCLK,
            if matches!(wiring, Wiring::MissingMclk) {
                0
            } else {
                1
            },
            false,
        ),
        (
            GCLK,
            if matches!(wiring, Wiring::MissingGclk) {
                0
            } else {
                1 << 6
            },
            false,
        ),
        (
            PORT + 0x36,
            if matches!(wiring, Wiring::WrongMux) {
                0x30
            } else {
                0x20
            },
            true,
        ),
        (PORT + 0x37, 0x20, true),
        (PORT + 0x4d, 1, true),
        (PORT + 0x4f, 1, true),
        (PORT, (1 << 7) | (1 << 5), false),
        (PORT + 0x18, 1 << 7, false), // CS inactive before enabling SPI
        (SPI, MASTER, false),
        (SPI + 4, 0, false), // receive disabled: no unused RX FIFO overflow
        (SPI + 0x0c, 1, true),
        (SPI, MASTER | 2, false),
    ] {
        guest.store(addr, value, byte);
    }
    guest.command(0x3a, &[3], wiring);
    if let Some(commands) = commands {
        for command in commands.as_array().unwrap() {
            let opcode = u8::try_from(command["opcode"].as_u64().unwrap()).unwrap();
            let mut args: Vec<u8> = command["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| u8::try_from(v.as_u64().unwrap()).unwrap())
                .collect();
            if truncate_lut && opcode == 0x2d {
                assert_eq!(args.len(), 128);
                args.pop(); // Named guest-stream negative, not host RAM mutation.
            }
            guest.command(opcode, &args, wiring);
        }
    } else {
        // Deliberately non-linear/channel-distinct six-bit LUT, not a linear ramp.
        let lut: [u8; 128] = std::array::from_fn(|i| ((i * 19 + i / 32 * 7 + 11) & 63) as u8);
        guest.command(0x2d, &lut, wiring);
        if let Some(swapped) = rectangular_axes_swapped {
            // Independent literal trace for CODAL setAddrWindow(7, 11, 3, 2):
            // CASET uses y..y+h-1; RASET uses x..x+w-1. This is controller
            // memory at MADCTL=0, NOT the deployed module's coordinate system.
            guest.command(0x2a, &[0, 11, 0, if swapped { 13 } else { 12 }], wiring);
            guest.command(0x2b, &[0, 7, 0, if swapped { 8 } else { 9 }], wiring);
            // Ordinary, non-doubled indexed source bytes 21 43 65: lower nibble
            // first, each index repeated into R/G/B. Literal expected wire bytes,
            // not generated by the emulator codec or its address mapping helper.
            // CS pauses after EVERY byte split both packed pixels and parameters.
            guest.command(
                0x2c,
                &[0x11, 0x12, 0x22, 0x33, 0x34, 0x44, 0x55, 0x56, 0x66],
                wiring,
            );
        } else {
            guest.command(0x2a, &[0, 0, 0, 1], wiring);
            guest.command(0x2b, &[0, 0, 0, 1], wiring);
            guest.command(0x2c, &[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc], wiring);
        }
    }
    if controls.is_some() {
        guest.command(0x11, &[], wiring);
        guest.command(0x29, &[], wiring);
    }
    if matches!(controls, Some(Controls::ResetWithSpiClockOff)) {
        guest.store(MCLK, 0, false);
        guest.store(PORT_A + 0x14, 1, false);
    }
    guest
}

#[test]
fn st7735_guest_gpio_controls_use_driven_port_pads_and_reset_without_spi_clock() {
    for legacy in [true, false] {
        for controls in [
            Controls::Released,
            Controls::BacklightOff,
            Controls::HeldReset,
            Controls::UndrivenReset,
            Controls::MuxedReset,
            Controls::WrongPort,
            Controls::ResetWithSpiClockOff,
        ] {
            let mut machine = Machine::new(CortexM::new(), board_with_controls(legacy, true));
            machine
                .load_firmware(&guest_with_controls(Wiring::Correct, Some(controls)))
                .unwrap();
            for _ in 0..50000 {
                machine.step().unwrap();
                if machine.bus.read_u32(0x20000000).unwrap() == MARKER {
                    break;
                }
            }
            assert_eq!(
                machine.bus.read_u32(0x20000000).unwrap(),
                MARKER,
                "{controls:?}, legacy={legacy}"
            );
            let frame = artifact(&machine.bus);
            let painted = matches!(
                controls,
                Controls::Released | Controls::BacklightOff | Controls::ResetWithSpiClockOff
            );
            assert_eq!(
                frame.meta["known_pixels"],
                if painted { 4 } else { 0 },
                "{controls:?}, legacy={legacy}"
            );
            assert_eq!(frame.bytes.is_some(), painted);
            assert_eq!(
                frame.meta["lit"],
                matches!(controls, Controls::Released),
                "{controls:?}, legacy={legacy}"
            );
            match controls {
                Controls::UndrivenReset | Controls::MuxedReset | Controls::WrongPort => {
                    assert!(frame.meta["reset_asserted"].is_null())
                }
                Controls::HeldReset | Controls::ResetWithSpiClockOff => {
                    assert_eq!(frame.meta["reset_asserted"], true)
                }
                _ => assert_eq!(frame.meta["reset_asserted"], false),
            }
            if matches!(controls, Controls::BacklightOff) {
                // Explicit host-MMIO diagnostic after the guest: visibility
                // changes on a GPIO write without a tick or RAM replacement.
                machine.bus.write_u32(PORT_A + 0x18, 2).unwrap();
                let lit = artifact(&machine.bus);
                assert_eq!(lit.meta["lit"], true);
                assert_eq!(lit.bytes, frame.bytes);
            }
            if matches!(controls, Controls::ResetWithSpiClockOff) {
                assert_eq!(
                    frame.meta["colmod"], 6,
                    "reset restored depth while SPI was gated"
                );
            }
        }
    }
}

#[test]
fn st7735_sam_known_pad_rejects_high_z_mux_and_contention_not_output_latch() {
    let mut bus = board_with_controls(false, true);
    bus.write_u32(PORT_A + 0x18, 1).unwrap();
    assert_eq!(DevicePins::known_pad_bit(&bus, PORT_A + 0x10, 0), None);
    bus.write_u32(PORT_A, 1).unwrap();
    assert_eq!(
        DevicePins::known_pad_bit(&bus, PORT_A + 0x10, 0),
        Some(true)
    );
    bus.write_u8(PORT_A + 0x40, 1).unwrap();
    assert_eq!(DevicePins::known_pad_bit(&bus, PORT_A + 0x10, 0), None);
    bus.write_u8(PORT_A + 0x40, 0).unwrap();
    let idx = bus
        .peripherals
        .iter()
        .position(|p| p.base == PORT_A)
        .unwrap();
    assert!(bus.set_peripheral_gpio_input(idx, 0, false));
    assert_eq!(DevicePins::known_pad_bit(&bus, PORT_A + 0x10, 0), None);
    // External drive propagation is NOT implemented by this GPIO-store
    // observer. A subsequent GPIO store samples the now-contended pad.
    bus.write_u32(PORT_A + 0x18, 1).unwrap();
    assert!(artifact(&bus).meta["reset_asserted"].is_null());
}

fn artifact(bus: &SystemBus) -> Artifact {
    bus.device_artifact_at(
        "sercom4",
        None,
        &[artifact_format::RGB888],
        "panel",
        &InspectOpts {
            include_bytes: true,
            peripheral: None,
        },
    )
    .expect("attached prototype evidence")
}

fn run(wiring: Wiring, legacy: bool) -> (u32, Artifact) {
    let mut machine = Machine::new(CortexM::new(), board(legacy));
    machine.load_firmware(&guest(wiring)).unwrap();
    for _ in 0..50000 {
        machine.step().unwrap();
        if machine.bus.read_u32(0x20000000).unwrap() == MARKER {
            break;
        }
    }
    (
        machine.bus.read_u32(0x20000000).unwrap(),
        artifact(&machine.bus),
    )
}

#[test]
fn st7735_guest_paints_packed_rgb444_through_sam_spi_and_real_gpio_cs_dc() {
    for legacy in [true, false] {
        let (marker, frame) = run(Wiring::Correct, legacy);
        assert_eq!(marker, MARKER, "legacy={legacy}: guest did not finish");
        assert_eq!(frame.meta["known_pixels"], 4);
        assert_eq!(frame.meta["unknown_pixels"], 0);
        // Independent expected table addresses for the authored packed stream.
        let addresses = [1, 34, 99, 4, 37, 102, 7, 40, 105, 10, 43, 108];
        let expected: Vec<u8> = addresses
            .into_iter()
            .map(|i| {
                let v = ((i * 19 + i / 32 * 7 + 11) & 63) as u8;
                (v << 2) | (v >> 4)
            })
            .collect();
        assert_eq!(
            frame.bytes.as_deref(),
            Some(expected.as_slice()),
            "legacy={legacy}"
        );
        assert_eq!(
            artifact_region_ink(
                artifact_format::RGB888,
                &frame.meta,
                frame.bytes.as_deref().unwrap(),
                PixelRegion {
                    x: 0,
                    y: 0,
                    w: 2,
                    h: 2
                }
            )
            .unwrap(),
            (4, 4)
        );
    }
}

#[test]
fn st7735_guest_clock_mux_cs_and_dc_negatives_do_not_paint_placeholder_pixels() {
    for legacy in [true, false] {
        for wiring in [
            Wiring::MissingMclk,
            Wiring::MissingGclk,
            Wiring::WrongMux,
            Wiring::CsHigh,
            Wiring::DcHigh,
        ] {
            let (marker, frame) = run(wiring, legacy);
            assert_eq!(frame.meta["known_pixels"], 0, "{wiring:?}, legacy={legacy}");
            assert_eq!(frame.meta["unknown_pixels"], 4);
            assert!(frame.bytes.is_none());
            if matches!(wiring, Wiring::CsHigh | Wiring::DcHigh) {
                assert_eq!(
                    marker, MARKER,
                    "wire negative must not fake a controller stall"
                );
            } else {
                assert_ne!(marker, MARKER, "clock/mux negative cannot pass TXC polling");
            }
        }
    }
}

/// Source reference (not a compiled or executed production driver):
/// https://github.com/lancaster-university/codal-core/blob/312ae57e0b31f5b9df07a81e9d846945828e3c5a/source/drivers/ST7735.cpp
/// setAddrWindow, sendBytes and ordinary expPalette establish this literal
/// shape. The nonlinear LUT is independently authored, not CODAL's palette
/// upload. No deployed CF2, actual module, DMA, BGR or orientation claim.
#[test]
fn st7735_guest_rectangular_indexed_shape_has_independent_nonzero_window_pixels() {
    for legacy in [true, false] {
        for swapped in [false, true] {
            let bus = board_with_window(
                legacy,
                false,
                GlassWindow {
                    col_offset: 11,
                    row_offset: 7,
                    cols: 2,
                    rows: 3,
                },
            );
            let mut machine = Machine::new(CortexM::new(), bus);
            machine
                .load_firmware(&guest_with_stream(Wiring::Correct, None, Some(swapped)))
                .unwrap();
            for _ in 0..50000 {
                machine.step().unwrap();
                if machine.bus.read_u32(0x20000000).unwrap() == MARKER {
                    break;
                }
            }
            assert_eq!(machine.bus.read_u32(0x20000000).unwrap(), MARKER);
            let frame = artifact(&machine.bus);
            if swapped {
                // Deliberately wrong 3-column/2-row window completes SPI but
                // misses the final physical row. Unknown pixels withhold bytes.
                assert_eq!(frame.meta["known_pixels"], 4, "legacy={legacy}");
                assert_eq!(frame.meta["unknown_pixels"], 2);
                assert!(frame.bytes.is_none());
                continue;
            }
            assert_eq!(frame.meta["known_pixels"], 6);
            assert_eq!(frame.meta["unknown_pixels"], 0);
            // Literal table addresses for row-major controller memory:
            // rows [1,2], [3,4], [5,6], each channel in its separate LUT block.
            let addresses = [
                1, 33, 97, 2, 34, 98, 3, 35, 99, 4, 36, 100, 5, 37, 101, 6, 38, 102,
            ];
            let expected: Vec<u8> = addresses
                .into_iter()
                .map(|i| {
                    let v = ((i * 19 + i / 32 * 7 + 11) & 63) as u8;
                    (v << 2) | (v >> 4)
                })
                .collect();
            assert_eq!(frame.bytes.as_deref(), Some(expected.as_slice()));
        }
    }
}

#[test]
fn st7735_guest_executes_captured_codal_commands_with_exact_palette_and_lut_negative() {
    // Byte-preserved official host-driver capture; NOT ARM CODAL execution.
    let trace: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/st7735-codal-host-trace.json")).unwrap();
    assert_eq!(trace["schema"], "labwired.st7735.codal-host-trace.v1");
    assert_eq!(
        trace["sourcePin"],
        "312ae57e0b31f5b9df07a81e9d846945828e3c5a"
    );
    assert_eq!(
        trace["inputSha256"]["source/drivers/ST7735.cpp"],
        "d8aafbdd338c9501c314f33983eb1ceb1229bc0acfcd10c28a4d2527d756c805"
    );
    assert_eq!(
        trace["inputSha256"]["inc/drivers/ST7735.h"],
        "41263645d09fe352ac21ddf72d3aa762c30465f90388ccad18a5775a899f5f23"
    );
    let cases = trace["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    // Independent physical-memory colours for indices 1..10, including known
    // black from unused LUT entries. No codec/mapping/palette helper is called.
    let expected = [
        255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 255, 0, 255, 0, 255, 255, 255, 255, 255, 16,
        52, 85, 0, 0, 0, 0, 0, 0,
    ];
    for (case, width) in cases.iter().zip([3u16, 4, 5]) {
        assert_eq!(case["width"], width);
        assert_eq!(case["height"], 2);
        for legacy in [true, false] {
            for truncated in [false, true] {
                let bus = board_with_window(
                    legacy,
                    false,
                    GlassWindow {
                        col_offset: 11,
                        row_offset: 7,
                        cols: 2,
                        rows: width,
                    },
                );
                let mut machine = Machine::new(CortexM::new(), bus);
                machine
                    .load_firmware(&guest_with_driver_trace(
                        Wiring::Correct,
                        None,
                        None,
                        Some(&case["commands"]),
                        truncated,
                    ))
                    .unwrap();
                for _ in 0..50000 {
                    machine.step().unwrap();
                    if machine.bus.read_u32(0x20000000).unwrap() == MARKER {
                        break;
                    }
                }
                assert_eq!(
                    machine.bus.read_u32(0x20000000).unwrap(),
                    MARKER,
                    "width={width}, legacy={legacy}, truncated={truncated}"
                );
                let frame = artifact(&machine.bus);
                if truncated {
                    assert_eq!(frame.meta["known_pixels"], 0);
                    assert_eq!(frame.meta["unknown_pixels"], u32::from(width) * 2);
                    assert!(frame.bytes.is_none());
                } else {
                    assert_eq!(frame.meta["known_pixels"], u32::from(width) * 2);
                    assert_eq!(frame.meta["unknown_pixels"], 0);
                    assert_eq!(
                        frame.bytes.as_deref(),
                        Some(&expected[..usize::from(width) * 6])
                    );
                }
            }
        }
    }
}

#[test]
fn st7735_guest_gm00_all_address_orientations_and_wrong_mv_negative() {
    // ST7735R v0.2 section 9.11.2: GM00 memory is 132x162. These are
    // independently tabulated address windows and physical row-major indices,
    // NOT generated with the model's orientation/mirroring helpers.
    // Each window targets fixed physical columns 11..12, rows 7..10.
    let vectors = [
        (
            0x00,
            [0, 11, 0, 12],
            [0, 7, 0, 10],
            [1, 2, 3, 4, 5, 6, 7, 8],
        ),
        (
            0x40,
            [0, 119, 0, 120],
            [0, 7, 0, 10],
            [2, 1, 4, 3, 6, 5, 8, 7],
        ),
        (
            0x80,
            [0, 11, 0, 12],
            [0, 151, 0, 154],
            [7, 8, 5, 6, 3, 4, 1, 2],
        ),
        (
            0xc0,
            [0, 119, 0, 120],
            [0, 151, 0, 154],
            [8, 7, 6, 5, 4, 3, 2, 1],
        ),
        (
            0x20,
            [0, 7, 0, 10],
            [0, 11, 0, 12],
            [1, 5, 2, 6, 3, 7, 4, 8],
        ),
        (
            0x60,
            [0, 7, 0, 10],
            [0, 119, 0, 120],
            [5, 1, 6, 2, 7, 3, 8, 4],
        ),
        (
            0xa0,
            [0, 151, 0, 154],
            [0, 11, 0, 12],
            [4, 8, 3, 7, 2, 6, 1, 5],
        ),
        (
            0xe0,
            [0, 151, 0, 154],
            [0, 119, 0, 120],
            [8, 4, 7, 3, 6, 2, 5, 1],
        ),
    ];
    // Literal expanded RGB666 colours for the unchanged captured palette.
    let colours: [[u8; 3]; 8] = [
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [255, 255, 0],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
        [16, 52, 85],
    ];
    let trace: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/st7735-codal-host-trace.json")).unwrap();
    let captured = &trace["cases"][1];
    assert_eq!(captured["width"], 4);
    assert_eq!(captured["height"], 2);
    for (madctl, columns, rows, indices) in vectors {
        let expected: Vec<u8> = indices
            .iter()
            .flat_map(|&index| colours[index - 1])
            .collect();
        for legacy in [false, true] {
            for wrong_mv in [false, true] {
                if wrong_mv && madctl != 0x20 {
                    continue;
                }
                // The original driver produced the palette and packed pixels.
                // MADCTL/windows are authored protocol inputs, not a capture of
                // that driver's rotated execution or deployed module settings.
                let mut commands = captured["commands"].as_array().unwrap().clone();
                assert_eq!(commands[0]["opcode"], 0x2a);
                assert_eq!(commands[1]["opcode"], 0x2b);
                commands[0]["data"] = serde_json::json!(columns);
                commands[1]["data"] = serde_json::json!(rows);
                commands.insert(
                    0,
                    serde_json::json!({
                        "opcode": 0x36,
                        "data": [if wrong_mv { 0 } else { madctl }],
                    }),
                );
                let commands = serde_json::Value::Array(commands);
                let mut machine = Machine::new(
                    CortexM::new(),
                    board_with_window(
                        legacy,
                        false,
                        GlassWindow {
                            col_offset: 11,
                            row_offset: 7,
                            cols: 2,
                            rows: 4,
                        },
                    ),
                );
                machine
                    .load_firmware(&guest_with_driver_trace(
                        Wiring::Correct,
                        None,
                        None,
                        Some(&commands),
                        false,
                    ))
                    .unwrap();
                for _ in 0..50000 {
                    machine.step().unwrap();
                    if machine.bus.read_u32(0x20000000).unwrap() == MARKER {
                        break;
                    }
                }
                assert_eq!(
                    machine.bus.read_u32(0x20000000).unwrap(),
                    MARKER,
                    "MADCTL={madctl:#04x}, legacy={legacy}, wrong_mv={wrong_mv}"
                );
                let frame = artifact(&machine.bus);
                if wrong_mv {
                    // Removing MV, without transposing the address windows,
                    // paints outside this fixed crop. Completion is still real.
                    assert_eq!(frame.meta["known_pixels"], 0);
                    assert_eq!(frame.meta["unknown_pixels"], 8);
                    assert!(frame.bytes.is_none());
                } else {
                    assert_eq!(frame.meta["known_pixels"], 8);
                    assert_eq!(frame.meta["unknown_pixels"], 0);
                    assert_eq!(
                        frame.bytes.as_deref(),
                        Some(expected.as_slice()),
                        "MADCTL={madctl:#04x}, legacy={legacy}"
                    );
                }
            }
        }
    }
}

#[test]
fn st7735_guest_retains_physical_ram_and_replaces_lut_only_for_future_pixels() {
    let trace: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/st7735-codal-host-trace.json")).unwrap();
    let captured = &trace["cases"][0];
    assert_eq!(captured["width"], 3);
    let original: Vec<u8> = vec![
        255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 255, 0, 255, 0, 255, 255,
    ];
    // Independent six-bit channel constants expanded to RGB888: 17,29,43.
    let replacement = [69, 117, 174];
    let mut lut = [17; 128];
    lut[32..96].fill(29);
    lut[96..].fill(43);
    for legacy in [false, true] {
        for omit_replacement in [false, true] {
            let mut guest = guest_trace_prefix(
                Wiring::Correct,
                Some(Controls::Released),
                None,
                Some(&captured["commands"]),
                false,
            );
            guest.store(0x20000000, 1, false);
            guest.command(0x36, &[0x40], Wiring::Correct);
            guest.store(0x20000000, 2, false);
            if !omit_replacement {
                guest.command(0x2d, &lut, Wiring::Correct);
            }
            guest.store(0x20000000, 3, false);
            // At MX=1, logical column120 maps to physical11; row7 is fixed.
            guest.command(0x2a, &[0, 120, 0, 120], Wiring::Correct);
            guest.command(0x2b, &[0, 7, 0, 7], Wiring::Correct);
            guest.command(0x2c, &[0x11, 0x10], Wiring::Correct);
            guest.store(0x20000000, 4, false);
            guest.command(0x01, &[], Wiring::Correct);
            guest.store(0x20000000, 5, false);
            // Software reset retains MX, RGB444 depth and the replacement LUT.
            guest.command(0x2a, &[0, 119, 0, 119], Wiring::Correct);
            guest.command(0x2b, &[0, 7, 0, 7], Wiring::Correct);
            guest.command(0x2c, &[0x22, 0x20], Wiring::Correct);
            guest.store(0x20000000, 6, false);
            // Real guest GPIO reset while SPI is gated; release before restart.
            guest.store(MCLK, 0, false);
            guest.store(PORT_A + 0x14, 1, false);
            guest.store(PORT_A + 0x18, 1, false);
            guest.store(MCLK, 1, false);
            guest.store(0x20000000, 7, false);
            // Hardware reset retained RAM, but the RGB444 LUT is now unknown.
            guest.command(0x3a, &[3], Wiring::Correct);
            guest.command(0x2a, &[0, 11, 0, 11], Wiring::Correct);
            guest.command(0x2b, &[0, 8, 0, 8], Wiring::Correct);
            guest.command(0x2c, &[0x33, 0x30], Wiring::Correct);
            guest.store(0x20000000, 8, false);
            guest.command(0x2d, &lut, Wiring::Correct);
            guest.store(0x20000000, 9, false);
            guest.command(0x2c, &[0x33, 0x30], Wiring::Correct);
            guest.store(0x20000000, 10, false);
            let mut machine = Machine::new(
                CortexM::new(),
                board_with_window(
                    legacy,
                    true,
                    GlassWindow {
                        col_offset: 11,
                        row_offset: 7,
                        cols: 2,
                        rows: 3,
                    },
                ),
            );
            machine.load_firmware(&guest.image()).unwrap();
            let mut expected = original.clone();
            for stage in 1..=if omit_replacement { 4 } else { 10 } {
                for _ in 0..50000 {
                    machine.step().unwrap();
                    if machine.bus.read_u32(0x20000000).unwrap() == stage {
                        break;
                    }
                }
                assert_eq!(
                    machine.bus.read_u32(0x20000000).unwrap(),
                    stage,
                    "stage={stage}, legacy={legacy}, omit_replacement={omit_replacement}"
                );
                let frame = artifact(&machine.bus);
                if stage == 4 && !omit_replacement {
                    expected[..3].copy_from_slice(&replacement);
                }
                if stage == 6 {
                    expected[3..6].copy_from_slice(&replacement);
                }
                if stage == 10 {
                    expected[6..9].copy_from_slice(&replacement);
                }
                assert_eq!(
                    frame.meta["colmod"],
                    if stage == 7 { 6 } else { 3 },
                    "stage={stage}, legacy={legacy}"
                );
                assert_eq!(
                    frame.meta["madctl"],
                    if (2..=6).contains(&stage) { 0x40 } else { 0 }
                );
                assert_eq!(frame.meta["reset_asserted"], false);
                assert_eq!(frame.meta["display_on"], stage <= 4);
                assert_eq!(frame.meta["awake"], stage <= 4);
                if stage == 8 || stage == 9 {
                    assert_eq!(frame.meta["known_pixels"], 5);
                    assert_eq!(frame.meta["unknown_pixels"], 1);
                    assert!(frame.bytes.is_none());
                } else {
                    assert_eq!(frame.meta["known_pixels"], 6);
                    assert_eq!(frame.meta["unknown_pixels"], 0);
                    assert_eq!(
                        frame.bytes.as_deref(),
                        Some(expected.as_slice()),
                        "stage={stage}, legacy={legacy}, omit_replacement={omit_replacement}"
                    );
                }
                if stage == 4 && omit_replacement {
                    let mut wanted = original.clone();
                    wanted[..3].copy_from_slice(&replacement);
                    assert_ne!(frame.bytes.as_deref(), Some(wanted.as_slice()));
                }
            }
        }
    }
}

#[test]
fn st7735_guest_invalid_ranges_preserve_known_and_unknown_corners_then_recover() {
    // Literal logical extents and physical corners for all GM00 orientations.
    let vectors = [
        (0x00, 132u16, 162u16, 131, 161),
        (0x40, 132, 162, 0, 161),
        (0x80, 132, 162, 131, 0),
        (0xc0, 132, 162, 0, 0),
        (0x20, 162, 132, 131, 161),
        (0x60, 162, 132, 0, 161),
        (0xa0, 162, 132, 131, 0),
        (0xe0, 162, 132, 0, 0),
    ];
    let window = |start: u16, end: u16| {
        let [a, b] = start.to_be_bytes();
        let [c, d] = end.to_be_bytes();
        [a, b, c, d]
    };
    let empty_commands = serde_json::json!([]);
    for (madctl, cols, rows, x, y) in vectors {
        for legacy in [false, true] {
            for seed_corner in [false, true] {
                let mut guest =
                    guest_trace_prefix(Wiring::Correct, None, None, Some(&empty_commands), false);
                guest.command(0x3a, &[6], Wiring::Correct);
                guest.command(0x36, &[madctl], Wiring::Correct);
                guest.command(0x2a, &window(cols - 1, cols - 1), Wiring::Correct);
                guest.command(0x2b, &window(rows - 1, rows - 1), Wiring::Correct);
                if seed_corner {
                    guest.command(0x2c, &[0, 0, 0xfc], Wiring::Correct);
                }
                guest.store(0x20000000, 1, false);
                // Oversized column must not turn an unknown pixel into known.
                guest.command(0x2a, &window(cols, cols), Wiring::Correct);
                guest.command(0x2c, &[0xfc, 0, 0], Wiring::Correct);
                guest.store(0x20000000, 2, false);
                guest.command(0x2a, &window(cols - 1, cols), Wiring::Correct);
                guest.command(0x2b, &window(rows - 1, rows), Wiring::Correct);
                guest.command(
                    0x2c,
                    &[0, 0xfc, 0, 0xfc, 0, 0, 0, 0, 0xfc, 0xfc, 0xfc, 0],
                    Wiring::Correct,
                );
                guest.store(0x20000000, 3, false);
                guest.command(0x2a, &window(u16::MAX, u16::MAX), Wiring::Correct);
                guest.command(0x2b, &window(rows - 1, rows - 1), Wiring::Correct);
                guest.command(0x2c, &[0xfc, 0, 0], Wiring::Correct);
                guest.store(0x20000000, 4, false);
                guest.command(0x2a, &window(cols - 1, cols - 2), Wiring::Correct);
                guest.command(0x2c, &[0xfc, 0, 0], Wiring::Correct);
                guest.store(0x20000000, 5, false);
                guest.command(0x3a, &[3], Wiring::Correct);
                guest.command(0x2a, &window(cols, cols), Wiring::Correct);
                guest.command(0x2c, &[0x12], Wiring::Correct); // incomplete RGB444
                guest.store(0x20000000, 6, false);
                guest.command(0x3a, &[6], Wiring::Correct); // discard fragment
                guest.command(0x2a, &window(cols - 1, cols - 1), Wiring::Correct);
                guest.command(0x2c, &[0x44, 0x74, 0xac], Wiring::Correct);
                guest.store(0x20000000, 7, false);
                let mut machine = Machine::new(
                    CortexM::new(),
                    board_with_window(
                        legacy,
                        false,
                        GlassWindow {
                            col_offset: x,
                            row_offset: y,
                            cols: 1,
                            rows: 1,
                        },
                    ),
                );
                machine.load_firmware(&guest.image()).unwrap();
                for stage in 1..=7 {
                    for _ in 0..50000 {
                        machine.step().unwrap();
                        if machine.bus.read_u32(0x20000000).unwrap() == stage {
                            break;
                        }
                    }
                    assert_eq!(
                        machine.bus.read_u32(0x20000000).unwrap(),
                        stage,
                        "MADCTL={madctl:#x}, stage={stage}, legacy={legacy}"
                    );
                    let frame = artifact(&machine.bus);
                    if stage <= 2 && !seed_corner {
                        assert_eq!(frame.meta["known_pixels"], 0);
                        assert_eq!(frame.meta["unknown_pixels"], 1);
                        assert!(frame.bytes.is_none());
                    } else {
                        let expected: &[u8] = match stage {
                            1 | 2 => &[0, 0, 255],
                            7 => &[69, 117, 174],
                            _ => &[0, 255, 0],
                        };
                        assert_eq!(frame.meta["known_pixels"], 1);
                        assert_eq!(frame.meta["unknown_pixels"], 0);
                        assert_eq!(
                            frame.bytes.as_deref(),
                            Some(expected),
                            "MADCTL={madctl:#x}, stage={stage}, legacy={legacy}, seed={seed_corner}"
                        );
                    }
                }
            }
        }
    }
}
