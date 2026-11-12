// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Manufacturer-protocol checks, including the real Bosch image bytes.
use labwired_core::peripherals::components::bmi270_spi::{Bmi270Spi, BMI270_SPI_KIT};
use labwired_core::peripherals::device::{SpiDevice, SpiSampling};
use labwired_core::peripherals::kit::{PeripheralKit, Transport};
use labwired_core::sim_input::SimInput;

const BASE: &[u8] = include_bytes!("fixtures/bmi270/base.bin");
const MAX_FIFO: &[u8] = include_bytes!("fixtures/bmi270/maximum-fifo.bin");

fn read(d: &mut Bmi270Spi, address: u8, count: usize) -> Vec<u8> {
    d.cs_select();
    d.transfer(address | 0x80);
    d.transfer(0);
    let bytes = (0..count).map(|_| d.transfer(0)).collect();
    d.cs_release();
    bytes
}
fn write(d: &mut Bmi270Spi, address: u8, bytes: &[u8]) {
    d.cs_select();
    d.transfer(address & 0x7f);
    for byte in bytes {
        d.transfer(*byte);
    }
    d.cs_release();
}
fn selected() -> Bmi270Spi {
    let mut d = Bmi270Spi::new("GPIO14").unwrap();
    d.cs_select();
    d.cs_release();
    d.advance_time_us(450);
    d
}
fn prepare(d: &mut Bmi270Spi) {
    write(d, 0x7c, &[0]);
    d.advance_time_us(450);
    write(d, 0x59, &[0]);
}
fn upload(d: &mut Bmi270Spi, image: &[u8]) {
    prepare(d);
    write(d, 0x5e, image);
    write(d, 0x59, &[1]);
}
fn initialized() -> Bmi270Spi {
    let mut d = selected();
    upload(&mut d, MAX_FIFO);
    d.advance_time_us(20_000);
    assert_eq!(read(&mut d, 0x21, 1), [1]);
    d
}

#[test]
fn chip_select_edge_and_both_startup_delays_gate_the_bus() {
    let mut d = Bmi270Spi::new("GPIO14").unwrap();
    d.advance_time_us(1000);
    assert_eq!(d.transfer(0x80), 0xff); // deselected
    assert_eq!(read(&mut d, 0, 1), [0xff]); // the first CS rise selects SPI
    d.advance_time_us(199);
    assert_eq!(read(&mut d, 0, 1), [0xff]);
    d.advance_time_us(1);
    assert_eq!(read(&mut d, 0, 1), [0x24]);

    let mut cold = Bmi270Spi::new("GPIO14").unwrap();
    cold.cs_select();
    cold.cs_release();
    cold.advance_time_us(449);
    assert_eq!(read(&mut cold, 0, 1), [0xff]);
    cold.advance_time_us(1);
    assert_eq!(read(&mut cold, 0, 1), [0x24]);
}

#[test]
fn spi_reads_have_one_dummy_byte_and_cs_terminates_each_frame() {
    let mut d = selected();
    d.cs_select();
    assert_eq!(d.transfer(0x80), 0);
    assert_eq!(d.transfer(0), 0); // not CHIP_ID
    assert_eq!(d.transfer(0), 0x24);
    d.cs_release();
    d.cs_select();
    d.transfer(0x40);
    d.cs_release(); // address without data
    assert_eq!(read(&mut d, 0x40, 2), [0xa8, 2]);
    write(&mut d, 0x40, &[0xac, 3, 0xed, 0]);
    assert_eq!(read(&mut d, 0x40, 4), [0xac, 3, 0xed, 0]);
    write(&mut d, 0, &[0xaa]);
    assert_eq!(read(&mut d, 0, 1), [0x24]);
    d.transfer(0x40);
    d.transfer(0xff); // no CS, no mutation
    assert_eq!(read(&mut d, 0x40, 1), [0xac]);
}

#[test]
fn exact_bosch_base_and_maximum_fifo_images_initialize_after_the_documented_bound() {
    for image in [BASE, MAX_FIFO] {
        let mut d = selected();
        upload(&mut d, image);
        assert_eq!(read(&mut d, 0x21, 1), [0]);
        d.advance_time_us(19_999);
        assert_eq!(read(&mut d, 0x21, 1), [0]);
        d.advance_time_us(1);
        assert_eq!(read(&mut d, 0x21, 1), [1]);
        assert_eq!(read(&mut d, 0x5e, image.len()), image);
    }
}

#[test]
fn truncated_corrupt_missing_and_repeated_images_do_not_report_init_ok() {
    let mut corrupt = MAX_FIFO.to_vec();
    corrupt[42] ^= 1;
    for image in [
        MAX_FIFO[..MAX_FIFO.len() - 1].to_vec(),
        corrupt,
        vec![],
        vec![0xb6; 8192],
    ] {
        let mut d = selected();
        upload(&mut d, &image);
        d.advance_time_us(30_000);
        assert_eq!(read(&mut d, 0x21, 1), [2]);
    }
    let mut d = initialized();
    write(&mut d, 0x59, &[1]);
    assert_eq!(read(&mut d, 0x21, 1), [2]);
}

#[test]
fn init_address_is_a_word_offset_and_chunk_readback_does_not_walk_into_cmd() {
    let mut d = selected();
    prepare(&mut d);
    for (offset, chunk) in BASE.chunks(128).enumerate() {
        let word_address = offset * 64;
        write(
            &mut d,
            0x5b,
            &[(word_address & 15) as u8, (word_address >> 4) as u8],
        );
        write(&mut d, 0x5e, chunk);
        assert_eq!(read(&mut d, 0x5e, chunk.len()), chunk);
    }
    write(&mut d, 0x59, &[1]);
    d.advance_time_us(20_000);
    assert_eq!(read(&mut d, 0x21, 1), [1]);
    assert_eq!(read(&mut d, 0, 1), [0x24]); // 0xb6 bytes in the blob never reset
}

#[test]
fn overwriting_one_chunk_cannot_pass_as_a_complete_upload() {
    let mut d = selected();
    prepare(&mut d);
    for _ in 0..64 {
        write(&mut d, 0x5e, &BASE[..128]);
    }
    write(&mut d, 0x59, &[1]);
    d.advance_time_us(20_000);
    assert_eq!(read(&mut d, 0x21, 1), [2]);
}

#[test]
fn upload_requires_advanced_power_save_exit_delay() {
    let mut d = selected();
    write(&mut d, 0x7c, &[0]);
    write(&mut d, 0x5e, MAX_FIFO);
    d.advance_time_us(450);
    write(&mut d, 0x59, &[1]);
    d.advance_time_us(20_000);
    assert_eq!(read(&mut d, 0x21, 1), [2]);
}

#[test]
fn reset_clears_initialization_and_requires_new_spi_selection() {
    let mut d = initialized();
    write(&mut d, 0x41, &[3]);
    write(&mut d, 0x7e, &[0xb6]);
    d.advance_time_us(449);
    assert_eq!(read(&mut d, 0, 1), [0xff]);
    d.advance_time_us(1);
    assert_eq!(read(&mut d, 0, 1), [0x24]);
    assert_eq!(read(&mut d, 0x21, 1), [0]);
    assert_eq!(read(&mut d, 0x41, 1), [2]);
    assert_eq!(read(&mut d, 0x5e, 10), [0; 10]);
}

#[test]
fn nominal_gyro_odr_drives_a_156_point_25_us_push_pull_interrupt_pulse() {
    let mut d = initialized();
    d.set_interrupt_pins(Some("GPIO13".into()), None);
    d.set_input("gx", 100.0).unwrap();
    write(&mut d, 0x42, &[0xed]); // 3200 Hz, period 312.5 us
    write(&mut d, 0x58, &[4]);
    write(&mut d, 0x53, &[0x0a]);
    assert_eq!(d.take_pin_drives(), [("GPIO13".into(), false)]);
    write(&mut d, 0x7d, &[2]);
    d.advance_time_us(312);
    assert_eq!(read(&mut d, 0x03, 1), [0x10]);
    assert!(d.take_pin_drives().is_empty());
    d.advance_time_us(1);
    assert_eq!(d.take_pin_drives(), [("GPIO13".into(), true)]);
    assert_eq!(read(&mut d, 0x03, 1), [0x50]);
    assert_eq!(read(&mut d, 0x12, 2), 1638i16.to_le_bytes());
    assert_eq!(read(&mut d, 0x03, 1), [0x10]);
    d.advance_time_us(155);
    assert!(d.take_pin_drives().is_empty());
    d.advance_time_us(1);
    assert_eq!(d.take_pin_drives(), [("GPIO13".into(), false)]);
    assert_eq!(read(&mut d, 0x1d, 1), [0x40]);
    assert_eq!(read(&mut d, 0x1d, 1), [0]);
    write(&mut d, 0x7d, &[0]);
    d.take_pin_drives();
    d.advance_time_us(1000);
    assert!(d.take_pin_drives().is_empty());
    assert_eq!(read(&mut d, 0x03, 1), [0x10]);
}

#[test]
fn latched_active_low_interrupt_waits_for_status_read_and_mapping_matters() {
    let mut d = initialized();
    d.set_interrupt_pins(Some("GPIO13".into()), None);
    write(&mut d, 0x55, &[1]);
    write(&mut d, 0x53, &[8]);
    write(&mut d, 0x42, &[0xe8]);
    write(&mut d, 0x7d, &[2]);
    assert_eq!(d.take_pin_drives(), [("GPIO13".into(), true)]);
    d.advance_time_us(10_000);
    assert!(d.take_pin_drives().is_empty()); // not mapped
    write(&mut d, 0x58, &[4]);
    assert_eq!(d.take_pin_drives(), [("GPIO13".into(), false)]);
    d.advance_time_us(1000);
    assert!(d.take_pin_drives().is_empty());
    assert_eq!(read(&mut d, 0x1d, 1), [0x40]);
    assert_eq!(d.take_pin_drives(), [("GPIO13".into(), true)]);
}

#[test]
fn enable_before_init_and_unpowered_devices_cannot_generate_samples() {
    let mut d = selected();
    write(&mut d, 0x7d, &[6]);
    d.advance_time_us(20_000);
    assert_eq!(read(&mut d, 0x03, 1), [0x10]);
    assert_eq!(read(&mut d, 0x0c, 12), [0; 12]);
    d.set_powered(false);
    d.cs_select();
    assert_eq!(d.transfer(0x80), 0xff);
    d.cs_release();
    d.advance_time_us(100_000);
    assert!(d.take_pin_drives().is_empty());
    d.set_powered(true);
    d.advance_time_us(1000);
    assert_eq!(read(&mut d, 0, 1), [0xff]);
    d.advance_time_us(200);
    assert_eq!(read(&mut d, 0, 1), [0x24]);
    assert_eq!(read(&mut d, 0x21, 1), [0]);
}

#[test]
fn transport_and_input_metadata_do_not_claim_a_breakout_or_feature_engine() {
    let mut d = selected();
    assert!(d.set_spi_mode(Some(1)).is_err());
    assert!(d.set_spi_mode(Some(2)).is_err());
    d.set_spi_mode(Some(3)).unwrap();
    assert_eq!(d.sampling(), SpiSampling::edge_mode(3));
    d.set_spi_mode(None).unwrap();
    assert_eq!(d.sampling(), SpiSampling::Byte);
    assert!(d.set_input("steps", 1.0).is_err());
    let meta = BMI270_SPI_KIT.metadata();
    assert_eq!(meta.device_type, "bmi270_spi");
    assert_eq!(meta.transport, Transport::Spi);
    assert_eq!(meta.inputs.len(), 7);
    assert!(meta.detail.contains("FIFO"));
    assert!(meta.config_keys.iter().any(|k| k.name == "int1_pin"));
}

#[test]
fn overlong_base_upload_and_reserved_gyro_range_are_not_accepted() {
    let mut d = selected();
    let mut bytes = BASE.to_vec();
    bytes.push(0);
    upload(&mut d, &bytes);
    d.advance_time_us(20_000);
    assert_eq!(read(&mut d, 0x21, 1), [2]);
    let mut d = initialized();
    write(&mut d, 0x43, &[7]);
    write(&mut d, 0x7d, &[2]);
    d.advance_time_us(20_000);
    assert_eq!(read(&mut d, 0x03, 1), [0x10]);
}

#[test]
fn reset_and_output_disable_do_not_invent_a_polarity_edge() {
    let mut d = initialized();
    d.set_interrupt_pins(Some("GPIO13".into()), None);
    write(&mut d, 0x53, &[0x0a]); // active-high, initially inactive
    assert_eq!(d.take_pin_drives(), [("GPIO13".into(), false)]);
    write(&mut d, 0x53, &[0]); // no output; old inactive remains low
    assert!(d.take_pin_drives().is_empty());
    write(&mut d, 0x53, &[0x0a]);
    d.take_pin_drives();
    write(&mut d, 0x7e, &[0xb6]);
    assert!(d.take_pin_drives().is_empty());
    d.advance_time_us(100_000);
    assert!(d.take_pin_drives().is_empty());
}

#[test]
fn registry_factory_attaches_native_spi_with_explicit_cs_and_power_state() {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use std::path::PathBuf;
    let chip_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/rp2040.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let make_manifest = |config: &str, connection: &str| -> SystemManifest {
        serde_yaml::from_str(&format!("name: bmi270-spi-factory\nchip: rp2040\nexternal_devices:\n  - id: U9\n    type: bmi270_spi\n    connection: {connection}\n    config: {config}\n")).unwrap()
    };
    let mut bus = SystemBus::from_config(
        &chip,
        &make_manifest("{cs_pin: GPIO14, int1_pin: GPIO13, powered: false}", "spi0"),
    )
    .unwrap();
    let index = bus.find_peripheral_index_by_name("spi0").unwrap();
    let spi = bus.peripherals[index]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<labwired_core::peripherals::rp2040::spi::Rp2040Spi>()
        .unwrap();
    let devices = &mut spi.attached_devices;
    assert_eq!(devices.len(), 1);
    assert_eq!(SpiDevice::component_id(&*devices[0]), Some("U9"));
    assert_eq!(devices[0].cs_pin(), "GPIO14");
    let d = devices[0]
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Bmi270Spi>()
        .unwrap();
    d.cs_select();
    d.cs_release();
    d.advance_time_us(1000);
    assert_eq!(read(d, 0, 1), [0xff]);
    for (config, connection, error) in [
        ("{}", "spi0", "cs_pin"),
        ("{cs_pin: GPIO14, spi_mode: 1}", "spi0", "modes 0 and 3"),
        ("{cs_pin: GPIO14, spi_mode: invalid}", "spi0", "spi_mode"),
        ("{cs_pin: GPIO14}", "uart0", "SPI"),
    ] {
        let err = SystemBus::from_config(&chip, &make_manifest(config, connection))
            .err()
            .expect("invalid attachment must fail");
        assert!(format!("{err:#}").contains(error), "{err:#}");
    }
}
