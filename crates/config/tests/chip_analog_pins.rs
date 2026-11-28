// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `analog_pins:` — the pad → ADC input map a chip descriptor may declare.
//!
//! The numbers are datasheet transcription, and a wrong one is worse than a
//! missing one: co-simulation routes `board.analog.<pad>_volts` through this
//! map, so a typo sends a model's voltage to a channel the firmware never
//! converts, silently. What CAN be checked without inventing silicon facts is
//! internal consistency — every entry names an ADC the descriptor actually has,
//! a pad whose GPIO port the descriptor actually has, and no two pads share one
//! input — plus the transcriptions this map was introduced with.

use labwired_config::{AdcPinFn, ChipDescriptor};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn chips_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips")
}

fn load(chip: &str) -> ChipDescriptor {
    let path = chips_dir().join(format!("{chip}.yaml"));
    ChipDescriptor::from_file(&path).unwrap_or_else(|e| panic!("load {chip}: {e}"))
}

fn all_chips() -> Vec<String> {
    let mut chips: Vec<String> = std::fs::read_dir(chips_dir())
        .expect("read configs/chips")
        .filter_map(|e| {
            let path = e.expect("dir entry").path();
            (path.extension()? == "yaml").then(|| path.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();
    chips.sort();
    chips
}

#[test]
fn every_analog_pin_names_a_real_adc_and_a_real_port() {
    let problems: Vec<_> = all_chips()
        .into_iter()
        .flat_map(|chip| analog_pin_problems(&chip, &load(&chip)))
        .collect();
    assert!(
        problems.is_empty(),
        "analog_pins problems:\n{}",
        problems.join("\n")
    );
}

fn analog_pin_problems(chip: &str, desc: &ChipDescriptor) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: BTreeMap<(String, u8), String> = BTreeMap::new();
    for (
        pad,
        AdcPinFn {
            peripheral,
            channel,
        },
    ) in &desc.analog_pins
    {
        match desc.peripherals.iter().find(|p| &p.id == peripheral) {
            None => problems.push(format!(
                "{chip}: {pad} names ADC '{peripheral}', not declared"
            )),
            Some(p)
                if !matches!(
                    p.r#type.as_str(),
                    "adc"
                        | "nrf52840_saadc"
                        | "avr_adc"
                        | "ra_adc12"
                        | "rp2040_adc"
                        | "samd21_adc"
                        | "esp32_sar_adc"
                        | "esp32c3_apb_saradc"
                        | "esp32s3_sens"
                ) =>
            {
                problems.push(format!(
                    "{chip}: {pad} names '{peripheral}', which is type '{}', not adc",
                    p.r#type
                ))
            }
            Some(_) => {}
        }
        // Validate BOTH alphaport STM32 pads and Nordic P<port>.<bit>.
        // Numeric labels are only accepted for the actual Nordic ADC IP.
        let nordic = desc
            .peripherals
            .iter()
            .any(|p| &p.id == peripheral && p.r#type == "nrf52840_saadc");
        let avr = desc
            .peripherals
            .iter()
            .any(|p| &p.id == peripheral && p.r#type == "avr_adc");
        let ra = desc
            .peripherals
            .iter()
            .any(|p| &p.id == peripheral && p.r#type == "ra_adc12");
        let rp2040 = desc
            .peripherals
            .iter()
            .any(|p| &p.id == peripheral && p.r#type == "rp2040_adc");
        let samd21 = desc
            .peripherals
            .iter()
            .any(|p| &p.id == peripheral && p.r#type == "samd21_adc");
        let esp32_sar = desc
            .peripherals
            .iter()
            .any(|p| &p.id == peripheral && p.r#type == "esp32_sar_adc");
        let esp32c3_sar = desc
            .peripherals
            .iter()
            .any(|p| &p.id == peripheral && p.r#type == "esp32c3_apb_saradc");
        let esp32s3_sens = desc
            .peripherals
            .iter()
            .any(|p| &p.id == peripheral && p.r#type == "esp32s3_sens");
        if esp32s3_sens {
            // ESP32-S3 datasheet analog-function table: ADC1_CH0..CH9 are
            // GPIO1..GPIO10, so the channel is the GPIO number minus one.
            // ADC2 (GPIO11..GPIO20) is excluded: the model's SAR2 one-shot
            // returns a fixed placeholder, not the pad level.
            match esp_gpio_number(pad) {
                Some(_) if *channel >= 10 => problems.push(format!(
                    "{chip}: {pad} names channel {channel}; S3 SAR ADC1 has 10 inputs"
                )),
                Some(gpio) if gpio != *channel + 1 => problems.push(format!(
                    "{chip}: {pad} must be GPIO{} for S3 SAR ADC1 channel {channel}",
                    *channel + 1
                )),
                Some(_) => {}
                None => problems.push(format!("{chip}: {pad} is not a GPIO<n> ESP32-S3 pad label")),
            }
        } else if esp32c3_sar {
            // The C3 APB_SARADC one-shot engine also serves the C5, C6 and C61.
            // Each chip's ADC1 pads come from its datasheet analog-function
            // table (see `esp32c3_family_adc1_gpio`). ADC2 pads are excluded:
            // the model's SAR2 one-shot shares the SAR1 channel slots, so an
            // ADC2 entry would alias an ADC1 pad.
            let Some(pads) = esp32c3_family_adc1_gpio(&desc.name) else {
                problems.push(format!(
                    "{chip}: {} runs esp32c3_apb_saradc but has no ADC1 pad table \
                     in esp32c3_family_adc1_gpio",
                    desc.name
                ));
                continue;
            };
            match esp_gpio_number(pad) {
                Some(gpio) => match pads.get(usize::from(*channel)) {
                    Some(&expected) if expected != gpio => problems.push(format!(
                        "{chip}: {pad} must be GPIO{expected} for {} SAR ADC1 channel {channel}",
                        desc.name
                    )),
                    Some(_) => {}
                    None => problems.push(format!(
                        "{chip}: {pad} names channel {channel}; {} SAR ADC1 has {} model inputs",
                        desc.name,
                        pads.len()
                    )),
                },
                None => problems.push(format!("{chip}: {pad} is not a GPIO<n> ESP32 pad label")),
            }
        } else if esp32_sar {
            // Classic ESP32 datasheet Table 2-2 (ESP32 Series Datasheet,
            // "Peripheral Pin Configurations"): SAR ADC1 CH0..CH7 are the fixed
            // pads GPIO36, 37, 38, 39, 32, 33, 34, 35. The pads have no GPIO
            // matrix choice, so the channel decides the pad exactly.
            match esp_gpio_number(pad) {
                Some(gpio) => match ESP32_ADC1_GPIO.get(usize::from(*channel)) {
                    Some(&expected) if expected != gpio => problems.push(format!(
                        "{chip}: {pad} must be GPIO{expected} for SAR ADC1 channel {channel}"
                    )),
                    Some(_) => {}
                    None => problems.push(format!(
                        "{chip}: {pad} names channel {channel}; SAR ADC1 has 8 inputs"
                    )),
                },
                None => problems.push(format!("{chip}: {pad} is not a GPIO<n> ESP32 pad label")),
            }
        } else if ra {
            // `ra_adc12` has one data register per channel, ADDR0-ADDR28, so a
            // channel number above 28 addresses nothing (see the `CHANNELS`
            // constant in crates/core/src/peripherals/ra_adc12.rs).
            if u64::from(*channel) >= 29 {
                problems.push(format!(
                    "{chip}: {pad} is outside {peripheral}'s 29 channels"
                ));
            }
            // Renesas RA names its ports `port<n>` and its pads `P<port><bit>`
            // with both numbers fixed-width (`P014` is port 0 bit 14, `P101`
            // is port 1 bit 0). A wrong split would route a solved node into a
            // completely different pad, so the label must be exactly one port
            // digit plus two bit digits and the port must be declared.
            match ra_pad_port_and_bit(pad) {
                Some((port, bit)) => match desc
                    .peripherals
                    .iter()
                    .find(|p| p.id == port && p.r#type == "gpio")
                {
                    Some(p) => {
                        let count = p
                            .config
                            .get("num_pins")
                            .and_then(serde_yaml::Value::as_u64)
                            .unwrap_or(16);
                        if u64::from(bit) >= count {
                            problems
                                .push(format!("{chip}: {pad} is outside {port}'s {count} pins"));
                        }
                    }
                    None => problems.push(format!(
                        "{chip}: {pad} is on {port}, not a declared RA GPIO"
                    )),
                },
                None => problems.push(format!(
                    "{chip}: {pad} is not a P<port><bit:02> RA pad label"
                )),
            }
        } else if avr {
            // AVR names its GPIO blocks `port<letter>` and its converter
            // `avr_adc`; a few inputs are dedicated die pads with no GPIO, so
            // their pad label is the channel name (`ADC6`).
            // `channels` is the width of the `avr_adc` input window, so this
            // bounds the window, not the number of bonded analog pads.
            let channels = desc
                .peripherals
                .iter()
                .find(|p| &p.id == peripheral)
                .and_then(|p| p.config.get("channels"))
                .and_then(serde_yaml::Value::as_u64)
                .unwrap_or(8);
            if u64::from(*channel) >= channels {
                problems.push(format!(
                    "{chip}: {pad} is outside {peripheral}'s {channels} channels"
                ));
            }
            match avr_pad_port_and_bit(pad) {
                Some((port, bit)) => match desc
                    .peripherals
                    .iter()
                    .find(|p| p.id == port && p.r#type == "avr_gpio")
                {
                    Some(p) => {
                        let count = p
                            .config
                            .get("num_pins")
                            .and_then(serde_yaml::Value::as_u64)
                            .unwrap_or(8);
                        if u64::from(bit) >= count {
                            problems
                                .push(format!("{chip}: {pad} is outside {port}'s {count} pins"));
                        }
                    }
                    None => problems.push(format!(
                        "{chip}: {pad} is on {port}, not a declared AVR GPIO"
                    )),
                },
                None => match dedicated_adc_channel(pad) {
                    Some(pin) if pin != *channel => problems.push(format!(
                        "{chip}: dedicated input {pad} must name channel {pin}, not {channel}"
                    )),
                    Some(_) => {}
                    None => problems.push(format!(
                        "{chip}: {pad} is not a P<port><bit> or ADC<n> label"
                    )),
                },
            }
        } else if rp2040 {
            // RP2040 datasheet §4.9.2 and RP2350 datasheet §12.5: the SAR
            // converter's inputs 0..3 are the pads GPIO26..GPIO29 in order, and
            // input 4 is the on-die temperature sensor, which `CS.TS_EN`
            // selects. There is no pad behind input 4, so a descriptor that
            // names it would let a board net bias the die sensor.
            match rp2040_pad_bit(pad) {
                Some(bit) if *channel < 4 => {
                    let expected_bit = 26 + u32::from(*channel);
                    if u32::from(bit) != expected_bit {
                        problems.push(format!(
                            "{chip}: {pad} must be GPIO{expected_bit} for ADC channel {channel}"
                        ));
                    }
                }
                Some(_) => problems.push(format!(
                    "{chip}: {pad} names channel {channel}; only inputs 0..3 are pads, \
                     input 4 is the on-die temperature sensor"
                )),
                None => problems.push(format!(
                    "{chip}: {pad} is not a GP<n> pad label (the RP2040/RP2350A pinout form)"
                )),
            }
        } else if samd21 {
            // SAMD21 pads are `PA<n>` / `PB<n>` and the two PORT groups are
            // declared as `gpio_a` / `gpio_b` (one LabWired port is one PORT
            // GROUP, not one package bank). The AINn input number is checked
            // against the CMSIS table by the core's end-to-end test; here the
            // label, the owning port and the 20-input window are structural.
            if u64::from(*channel) >= 20 {
                problems.push(format!(
                    "{chip}: {pad} is outside {peripheral}'s 20 AIN inputs"
                ));
            }
            match samd21_pad_port_and_bit(pad) {
                Some((port, bit)) => match desc
                    .peripherals
                    .iter()
                    .find(|p| p.id == port && p.r#type == "gpio")
                {
                    Some(p) => {
                        let count = p
                            .config
                            .get("num_pins")
                            .and_then(serde_yaml::Value::as_u64)
                            .unwrap_or(32);
                        if u64::from(bit) >= count {
                            problems
                                .push(format!("{chip}: {pad} is outside {port}'s {count} pins"));
                        }
                    }
                    None => problems.push(format!(
                        "{chip}: {pad} is on {port}, not a declared SAMD21 GPIO"
                    )),
                },
                None => problems.push(format!(
                    "{chip}: {pad} is not a P<A|B><bit> SAMD21 pad label"
                )),
            }
        } else {
            match pad_port_and_bit(pad, nordic) {
                Some((port, bit)) => {
                    match desc.peripherals.iter().find(|p| {
                        p.id == port && matches!(p.r#type.as_str(), "gpio" | "stm32f4_gpio")
                    }) {
                        Some(p) => {
                            let count = p
                                .config
                                .get("num_pins")
                                .and_then(serde_yaml::Value::as_u64)
                                .unwrap_or(if nordic { 32 } else { 16 });
                            if u64::from(bit) >= count {
                                problems.push(format!(
                                    "{chip}: {pad} is outside {port}'s {count} pins"
                                ));
                            }
                            if nordic
                                && (port != "gpio0"
                                    || [2, 3, 4, 5, 28, 29, 30, 31].get(*channel as usize)
                                        != Some(&bit))
                            {
                                problems.push(format!(
                                    "{chip}: {pad} is not Nordic AIN{channel}'s physical pad"
                                ));
                            }
                        }
                        None => problems
                            .push(format!("{chip}: {pad} is on {port}, not a declared GPIO")),
                    }
                }
                None => problems.push(format!("{chip}: {pad} is not a P<port><bit> label")),
            }
        }
        if let Some(other) = seen.insert((peripheral.clone(), *channel), pad.clone()) {
            problems.push(format!(
                "{chip}: {pad} and {other} both claim {peripheral} channel {channel}"
            ));
        }
    }
    problems
}

/// ADC1 pads, in channel order, for the chips that run the C3 APB_SARADC
/// model. ESP32-C3/C6: ADC1_CH0..CH4 are GPIO0..GPIO4 (C3/C6 datasheets,
/// analog functions; the C6's CH5/CH6 on GPIO5/GPIO6 wait for a per-chip
/// model channel count). ESP32-C61: ADC1_CH0..CH3 are GPIO1, GPIO3, GPIO4 and
/// GPIO5 (ESP-IDF components/soc/esp32c61/include/soc/adc_channel.h).
/// ESP32-C5: ADC1_CH0..CH5 are GPIO1..GPIO6 (ESP-IDF
/// components/soc/esp32c5/include/soc/adc_channel.h); CH5 waits for the same
/// per-chip model channel count as the C6. An unlisted chip returns `None`,
/// so a new chip on this model must add its own table.
fn esp32c3_family_adc1_gpio(chip_name: &str) -> Option<&'static [u8]> {
    match chip_name {
        "esp32c3" | "esp32c6" | "esp32c6fh4" => Some(&[0, 1, 2, 3, 4]),
        "esp32c5" => Some(&[1, 2, 3, 4, 5]),
        "esp32c61" => Some(&[1, 3, 4, 5]),
        _ => None,
    }
}

/// Classic ESP32 SAR ADC1 CH0..CH7 pads, in channel order.
const ESP32_ADC1_GPIO: [u8; 8] = [36, 37, 38, 39, 32, 33, 34, 35];

/// `GPIO34` -> 34. Espressif names pads by their GPIO number only.
fn esp_gpio_number(pad: &str) -> Option<u8> {
    let digits = pad.strip_prefix("GPIO")?;
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return None;
    }
    digits.parse().ok()
}

/// The two pad shapes an AVR descriptor may name: a `P<port><bit>` GPIO pad
/// (`PF7` -> `portf` bit 7), or a dedicated ADC die pad with no GPIO block.
fn avr_pad_port_and_bit(pad: &str) -> Option<(String, u8)> {
    let upper = pad.to_ascii_uppercase();
    let rest = upper.strip_prefix('P')?;
    let port = rest.chars().next()?;
    if !port.is_ascii_alphabetic() {
        return None;
    }
    let digits = &rest[1..];
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((
        format!("port{}", port.to_ascii_lowercase()),
        digits.parse().ok()?,
    ))
}

/// `ADC6`/`ADC7` on an ATmega328P are dedicated inputs, not Port C pads; the
/// label already carries the channel number.
fn dedicated_adc_channel(pad: &str) -> Option<u8> {
    pad.to_ascii_uppercase().strip_prefix("ADC")?.parse().ok()
}

fn pad_port_and_bit(pad: &str, nordic: bool) -> Option<(String, u8)> {
    let upper = pad.to_ascii_uppercase();
    let rest = upper.strip_prefix('P')?;
    if nordic {
        let (port, bit) = rest.split_once('.')?;
        if port.len() != 1
            || bit.len() != 2
            || !port.bytes().chain(bit.bytes()).all(|c| c.is_ascii_digit())
        {
            return None;
        }
        Some((
            format!("gpio{}", port.parse::<u8>().ok()?),
            bit.parse().ok()?,
        ))
    } else {
        let port = rest.chars().next()?;
        if !port.is_ascii_alphabetic() {
            return None;
        }
        let digits = &rest[1..];
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some((
            format!("gpio{}", port.to_ascii_lowercase()),
            digits.parse().ok()?,
        ))
    }
}

/// SAMD21 pad labels: `PA<n>` / `PB<n>`, where A/B is the PORT group and
/// `<n>` the bit inside it. The descriptor declares exactly those two groups
/// as `gpio_a` / `gpio_b`.
fn samd21_pad_port_and_bit(pad: &str) -> Option<(String, u8)> {
    let upper = pad.to_ascii_uppercase();
    let rest = upper.strip_prefix('P')?;
    let port = rest.chars().next()?;
    if !matches!(port, 'A' | 'B') {
        return None;
    }
    let digits = &rest[1..];
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((
        format!("gpio_{}", port.to_ascii_lowercase()),
        digits.parse().ok()?,
    ))
}

/// RP2040/RP2350A pad labels: `GP<n>` (`GP26` is the Pico pinout form the
/// descriptor's `pins:` block and the playground emit both use).
fn rp2040_pad_bit(pad: &str) -> Option<u8> {
    let upper = pad.to_ascii_uppercase();
    let digits = upper.strip_prefix("GP")?;
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// RA pad labels: `P<port><bit:02>`, e.g. `P014` -> port0 bit 14 and `P101` ->
/// port1 bit 0. Both numbers are fixed width in FSP's `R7FA4M1AB.h` pad macro
/// names (`BSP_IO_PORT_00_PIN_14`), so exactly one port digit and two bit
/// digits are accepted. `PA5`-style STM32 labels must never pass here.
fn ra_pad_port_and_bit(pad: &str) -> Option<(String, u8)> {
    let rest = pad.to_ascii_uppercase();
    let rest = rest.strip_prefix('P')?;
    if rest.len() != 3 || !rest.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let port = rest[..1].parse::<u8>().ok()?;
    let bit = rest[1..].parse::<u8>().ok()?;
    Some((format!("port{port}"), bit))
}

/// ArduinoCore-renesas 1.6.0 `variants/MINIMA`: A0-A5 are P014, P000, P001,
/// P002, P101, P100, and `pinmux.inc` gives their `PIN_ANALOG` channels as
/// 9, 0, 1, 2, 21, 22. A4/A5 land in ADANSA1, so this map exercises both
/// halves of the group-A selector.
#[test]
fn ra4m1_analog_pins_match_the_arduino_uno_r4_pinout() {
    let desc = load("ra4m1");
    let expected = [
        ("P014", 9u8),
        ("P000", 0),
        ("P001", 1),
        ("P002", 2),
        ("P101", 21),
        ("P100", 22),
    ];
    assert_eq!(desc.analog_pins.len(), expected.len());
    for (pad, channel) in expected {
        let mapping = &desc.analog_pins[pad];
        assert_eq!(mapping.peripheral, "adc0", "{pad} must name the UNO R4 ADC");
        assert_eq!(mapping.channel, channel, "{pad}");
    }
    assert!(analog_pin_problems("ra4m1", &desc).is_empty());
}

#[test]
fn ra_validation_rejects_bad_models_ports_channels_and_pads() {
    let original = load("ra4m1");
    for bad_pad in ["P14", "P0140", "PA14", "P0.14", "p014x"] {
        let mut desc = original.clone();
        let mapping = desc.analog_pins.remove("P014").unwrap();
        desc.analog_pins.insert(bad_pad.to_string(), mapping);
        assert!(
            !analog_pin_problems("bad", &desc).is_empty(),
            "accepted {bad_pad}"
        );
    }
    // Inside the label grammar, outside the port's 16 pins.
    let mut desc = original.clone();
    let mapping = desc.analog_pins.remove("P014").unwrap();
    desc.analog_pins.insert("P190".to_string(), mapping);
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // A channel with no ADDRn behind it: ra_adc12 stops at 28.
    let mut desc = original.clone();
    desc.analog_pins.get_mut("P014").unwrap().channel = 29;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // Two pads on one input.
    let mut desc = original.clone();
    desc.analog_pins.get_mut("P000").unwrap().channel = 9;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // A pad whose port is not declared.
    let mut desc = original.clone();
    desc.peripherals.retain(|p| p.id != "port0");
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // The peripheral is not a converter.
    let mut desc = original;
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "adc0")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

#[test]
fn nordic_analog_pins_match_physical_ain_assignment() {
    let desc = load("nrf52833");
    assert_eq!(desc.analog_pins.len(), 8);
    for (channel, pin) in [2, 3, 4, 5, 28, 29, 30, 31].into_iter().enumerate() {
        let mapping = &desc.analog_pins[&format!("P0.{pin:02}")];
        assert_eq!(mapping.peripheral, "saadc");
        assert_eq!(usize::from(mapping.channel), channel);
    }
    assert!(analog_pin_problems("nrf52833", &desc).is_empty());
}

#[test]
fn nordic_validation_rejects_bad_models_ports_channels_and_pads() {
    let original = load("nrf52833");
    for bad_pad in ["P0.99", "P9.05", "P0.06", "P0.5", "P0.05oops", "PA5"] {
        let mut desc = original.clone();
        let mapping = desc.analog_pins.remove("P0.05").unwrap();
        desc.analog_pins.insert(bad_pad.to_string(), mapping);
        assert!(
            !analog_pin_problems("bad", &desc).is_empty(),
            "accepted {bad_pad}"
        );
    }
    let mut desc = original.clone();
    desc.analog_pins.get_mut("P0.05").unwrap().channel = 8;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original.clone();
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "saadc")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original;
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "gpio0")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

/// The regular-input assignment shared by the F1/F4/F7 parts in-tree:
/// PA0..PA7 = IN0..IN7, PB0/PB1 = IN8/IN9, and on packages that bond them out,
/// PC0..PC5 = IN10..IN15.
fn f1_f4_assignment(with_pc0_to_pc5: bool) -> BTreeMap<String, AdcPinFn> {
    let mut pads: Vec<(String, u8)> = (0..8).map(|i| (format!("PA{i}"), i)).collect();
    pads.push(("PB0".to_string(), 8));
    pads.push(("PB1".to_string(), 9));
    if with_pc0_to_pc5 {
        pads.extend((0..6).map(|i| (format!("PC{i}"), 10 + i)));
    }
    pads.into_iter()
        .map(|(pad, channel)| {
            (
                pad,
                AdcPinFn {
                    peripheral: "adc1".to_string(),
                    channel,
                },
            )
        })
        .collect()
}

/// The 48-pin packages (F103C8 LQFP48, F401CD/F411CE UFQFPN48) have no
/// PC0..PC5 pads, so their map stops at IN9.
#[test]
fn f1_f4_f7_descriptors_carry_the_datasheet_assignment() {
    for (chip, with_pc) in [
        ("stm32f103", false),
        ("stm32f401cdu6", false),
        ("stm32f411ceu6", false),
        ("stm32f401", true),
        ("stm32f405", true),
        ("stm32f407", true),
        ("stm32f767", true),
    ] {
        assert_eq!(load(chip).analog_pins, f1_f4_assignment(with_pc), "{chip}");
    }
}

/// The L476 numbers its regular inputs from PC0, not PA0 (DS10198 pin definitions,
/// LQFP64): PC0..PC3 = IN1..IN4, PA0..PA7 = IN5..IN12, PC4/PC5 = IN13/IN14,
/// PB0/PB1 = IN15/IN16. A copy of the F1/F4 map would put PA0 on IN0.
#[test]
fn stm32l476_descriptor_carries_the_ds10198_assignment() {
    let mut pads: Vec<(String, u8)> = (0..4).map(|i| (format!("PC{i}"), 1 + i)).collect();
    pads.extend((0..8).map(|i| (format!("PA{i}"), 5 + i)));
    pads.extend([
        ("PC4".to_string(), 13),
        ("PC5".to_string(), 14),
        ("PB0".to_string(), 15),
        ("PB1".to_string(), 16),
    ]);
    let expected: BTreeMap<String, AdcPinFn> = pads
        .into_iter()
        .map(|(pad, channel)| {
            (
                pad,
                AdcPinFn {
                    peripheral: "adc1".to_string(),
                    channel,
                },
            )
        })
        .collect();
    assert_eq!(load("stm32l476").analog_pins, expected);
}

fn adc1_map(pads: &[(&str, u8)]) -> BTreeMap<String, AdcPinFn> {
    pads.iter()
        .map(|(pad, channel)| {
            (
                (*pad).to_string(),
                AdcPinFn {
                    peripheral: "adc1".to_string(),
                    channel: *channel,
                },
            )
        })
        .collect()
}

/// STM32G071RB LQFP64 (ST STM32_open_pin_data `STM32G071R(6-8-B)Tx.xml`):
/// PA0..PA7 = IN0..IN7, PB0..PB2 = IN8..IN10, PB10 = IN11, PB11 = IN15,
/// PB12 = IN16, PC4 = IN17, PC5 = IN18.
#[test]
fn stm32g071_descriptor_carries_the_lqfp64_assignment() {
    let mut pads: Vec<(String, u8)> = (0..8).map(|i| (format!("PA{i}"), i)).collect();
    pads.extend(
        [
            ("PB0", 8),
            ("PB1", 9),
            ("PB2", 10),
            ("PB10", 11),
            ("PB11", 15),
            ("PB12", 16),
            ("PC4", 17),
            ("PC5", 18),
        ]
        .map(|(pad, channel)| (pad.to_string(), channel)),
    );
    let pads: Vec<(&str, u8)> = pads.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    assert_eq!(load("stm32g071").analog_pins, adc1_map(&pads));
}

/// STM32C031C6 LQFP48 (ST STM32_open_pin_data `STM32C031C(4-6)Tx.xml`):
/// PA0..PA8 = IN0..IN8, PA11..PA14 = IN11..IN14, PB0 = IN17, PB1 = IN18. PB2
/// and PB10..PB12 (IN19..IN22) are absent on purpose: the `stm32g0` model
/// converts CHSEL0..18 only.
#[test]
fn stm32c031c6_descriptor_carries_the_lqfp48_assignment() {
    let mut pads: Vec<(String, u8)> = (0..9).map(|i| (format!("PA{i}"), i)).collect();
    pads.extend((11..15).map(|i| (format!("PA{i}"), i)));
    pads.extend([("PB0".to_string(), 17), ("PB1".to_string(), 18)]);
    let pads: Vec<(&str, u8)> = pads.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    let map = load("stm32c031c6").analog_pins;
    assert_eq!(map, adc1_map(&pads));
    for pad in ["PB2", "PB10", "PB11", "PB12"] {
        assert!(
            !map.contains_key(pad),
            "{pad} is past the model's CHSEL0..18"
        );
    }
}

/// STM32G431RB and STM32G474RE LQFP64 (ST STM32_open_pin_data
/// `STM32G431R(6-8-B)Tx.xml`, `STM32G474R(B-C-E)Tx.xml`): the ADC1 inputs are
/// PA0..PA3 = IN1..IN4, PB14 = IN5, PC0..PC3 = IN6..IN9, PB12 = IN11,
/// PB1 = IN12, PB11 = IN14, PB0 = IN15. PF0 (IN10) is absent: no GPIOF is
/// declared. Pads that reach only ADC2/ADC3/ADC5 are absent: only ADC1 is
/// instantiated.
#[test]
fn stm32g4_descriptors_carry_the_lqfp64_adc1_assignment() {
    let mut pads: Vec<(String, u8)> = (0..4).map(|i| (format!("PA{i}"), 1 + i)).collect();
    pads.push(("PB14".to_string(), 5));
    pads.extend((0..4).map(|i| (format!("PC{i}"), 6 + i)));
    pads.extend(
        [("PB12", 11), ("PB1", 12), ("PB11", 14), ("PB0", 15)]
            .map(|(pad, channel)| (pad.to_string(), channel)),
    );
    let pads: Vec<(&str, u8)> = pads.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    for chip in ["stm32g431rb", "stm32g474re"] {
        let map = load(chip).analog_pins;
        assert_eq!(map, adc1_map(&pads), "{chip}");
        for pad in ["PA4", "PA5", "PA6", "PA7", "PC4", "PC5", "PB2", "PB15"] {
            assert!(!map.contains_key(pad), "{chip}: {pad} is an ADC2-only pad");
        }
    }
}

/// STM32WB55RG VFQFPN68 (ST STM32_open_pin_data `STM32WB55RGVx.xml`):
/// PC0..PC3 = IN1..IN4, PA0..PA7 = IN5..IN12, PC4/PC5 = IN13/IN14,
/// PA8/PA9 = IN15/IN16. Unlike the L476, PB0/PB1 carry no ADC input.
#[test]
fn stm32wb55_descriptor_carries_the_vfqfpn68_assignment() {
    let mut pads: Vec<(String, u8)> = (0..4).map(|i| (format!("PC{i}"), 1 + i)).collect();
    pads.extend((0..8).map(|i| (format!("PA{i}"), 5 + i)));
    pads.extend(
        [("PC4", 13), ("PC5", 14), ("PA8", 15), ("PA9", 16)]
            .map(|(pad, channel)| (pad.to_string(), channel)),
    );
    let pads: Vec<(&str, u8)> = pads.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    let map = load("stm32wb55").analog_pins;
    assert_eq!(map, adc1_map(&pads));
    assert!(map.keys().all(|pad| !pad.starts_with("PB")));
}

/// STM32L031K6 LQFP32 (ST STM32_open_pin_data `STM32L031K(4-6)Tx.xml`):
/// PA0..PA7 = IN0..IN7, PB0 = IN8, PB1 = IN9.
#[test]
fn stm32l031k6_descriptor_carries_the_lqfp32_assignment() {
    let mut pads: Vec<(String, u8)> = (0..8).map(|i| (format!("PA{i}"), i)).collect();
    pads.extend([("PB0".to_string(), 8), ("PB1".to_string(), 9)]);
    let pads: Vec<(&str, u8)> = pads.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    assert_eq!(load("stm32l031k6").analog_pins, adc1_map(&pads));
}

/// Families whose assignment differs from the F1/F4 map were deliberately NOT
/// transcribed. An entry there must come from that part's own pin data, not
/// from this map.
#[test]
fn families_with_a_different_assignment_declare_nothing_yet() {
    for chip in ["stm32h563"] {
        assert!(
            load(chip).analog_pins.is_empty(),
            "{chip} gained analog_pins; check them against its own datasheet and update this test"
        );
    }
}

#[test]
fn a_descriptor_without_analog_pins_parses_to_an_empty_map() {
    let yaml = "name: \"bare\"\narch: \"arm\"\ncore: \"cortex-m4\"\n\
                flash: { base: 0, size: \"4KB\" }\n\
                ram: { base: 0x20000000, size: \"1KB\" }\nperipherals: []\n";
    let desc: ChipDescriptor = serde_yaml::from_str(yaml).expect("parse");
    assert!(desc.analog_pins.is_empty());
}

#[test]
fn an_analog_pin_entry_rejects_unknown_fields() {
    let yaml = "name: \"bad\"\narch: \"arm\"\ncore: \"cortex-m4\"\n\
                flash: { base: 0, size: \"4KB\" }\n\
                ram: { base: 0x20000000, size: \"1KB\" }\nperipherals: []\n\
                analog_pins:\n  PA0: { adc: \"adc1\", channel: 0 }\n";
    assert!(
        serde_yaml::from_str::<ChipDescriptor>(yaml).is_err(),
        "a misspelled key must fail loudly, not parse as a pad with no ADC"
    );
}

/// The AVR transcription: ATmega328P ADC0..ADC5 = PC0..PC5 plus the dedicated
/// ADC6/ADC7 TQFP pads, and ATmega32U4 ADC0..ADC7 = PF0..PF7, ADC8..ADC10 =
/// PD4/PD6/PD7 (PD5 has no ADC input) and ADC11..ADC13 = PB4..PB6. DS40002061B
/// Table 24-4 and DS7766 Tables 10-3/10-8/10-13/24-4.
#[test]
fn avr_descriptors_carry_the_datasheet_assignment() {
    let mega = load("atmega328p");
    assert_eq!(mega.analog_pins.len(), 8);
    for channel in 0..6u8 {
        assert_eq!(
            mega.analog_pins[&format!("PC{channel}")].channel,
            channel,
            "ATmega328P PC{channel} is ADC{channel}"
        );
    }
    for channel in [6u8, 7] {
        assert_eq!(
            mega.analog_pins[&format!("ADC{channel}")].channel,
            channel,
            "ATmega328P ADC{channel} is the dedicated input"
        );
    }
    assert!(analog_pin_problems("atmega328p", &mega).is_empty());

    let leonardo = load("atmega32u4");
    let assignment: [(&str, u8); 12] = [
        ("PF0", 0),
        ("PF1", 1),
        ("PF4", 4),
        ("PF5", 5),
        ("PF6", 6),
        ("PF7", 7),
        ("PD4", 8),
        ("PD6", 9),
        ("PD7", 10),
        ("PB4", 11),
        ("PB5", 12),
        ("PB6", 13),
    ];
    assert_eq!(leonardo.analog_pins.len(), assignment.len());
    for (pad, channel) in assignment {
        assert_eq!(
            leonardo.analog_pins[pad].channel, channel,
            "ATmega32U4 {pad} is ADC{channel}"
        );
    }
    assert!(
        !leonardo.analog_pins.contains_key("PD5"),
        "PD5 is XCK1/CTS on the ATmega32U4 and has no ADC input"
    );
    assert!(analog_pin_problems("atmega32u4", &leonardo).is_empty());
}

#[test]
fn avr_validation_rejects_bad_models_ports_channels_and_pads() {
    let original = load("atmega32u4");

    // A pad label must be a real AVR GPIO or a dedicated ADC<n> input.
    for bad_pad in ["PZ0", "PD8", "P7", "PA", "PD5x", "ADC14"] {
        let mut desc = original.clone();
        let mapping = desc.analog_pins.remove("PD4").unwrap();
        desc.analog_pins.insert(bad_pad.to_string(), mapping);
        assert!(
            !analog_pin_problems("bad", &desc).is_empty(),
            "accepted {bad_pad}"
        );
    }

    let mut desc = original.clone();
    desc.analog_pins.get_mut("PD4").unwrap().channel = 16;
    assert!(!analog_pin_problems("bad", &desc).is_empty());

    let mut desc = original.clone();
    desc.analog_pins.get_mut("PD6").unwrap().channel = 8;
    assert!(
        !analog_pin_problems("bad", &desc).is_empty(),
        "PD6 and PD4 cannot both claim ADC8"
    );

    let mut desc = original.clone();
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "adc")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());

    let mut desc = original.clone();
    desc.analog_pins.get_mut("PF7").unwrap().peripheral = "adc9".into();
    assert!(
        !analog_pin_problems("bad", &desc).is_empty(),
        "an AVR pad cannot name an ADC the descriptor does not declare"
    );

    let mut desc = original.clone();
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "portf")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());

    // A dedicated pad's label is its channel number, not just any number.
    let mut desc = original.clone();
    desc.analog_pins.insert(
        "ADC6".to_string(),
        AdcPinFn {
            peripheral: "adc".to_string(),
            channel: 7,
        },
    );
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

/// RP2040 datasheet §4.9.2: the SAR converter's pads are ADC0..ADC3 on
/// GPIO26..GPIO29; input 4 is the on-die temperature sensor. RP2350A keeps the
/// same four pads (the extra ADC4..ADC7 inputs belong to the 48-GPIO B
/// package). Every Pico-family descriptor must carry exactly that map, because
/// a missing entry is a board whose nets cannot reach the converter and a
/// wrong one silently sends a level to a channel firmware never converts.
#[test]
fn pico_family_analog_pins_match_the_datasheet_pads() {
    for chip in ["rp2040", "rp2040-feather", "rp2350", "rp2350-feather"] {
        let desc = load(chip);
        let expected = [("GP26", 0u8), ("GP27", 1), ("GP28", 2), ("GP29", 3)];
        assert_eq!(desc.analog_pins.len(), expected.len(), "{chip}");
        for (pad, channel) in expected {
            let mapping = &desc.analog_pins[pad];
            assert_eq!(mapping.peripheral, "adc", "{chip}: {pad}");
            assert_eq!(mapping.channel, channel, "{chip}: {pad}");
        }
        assert!(
            analog_pin_problems(chip, &desc).is_empty(),
            "{chip} must pass the pad/channel consistency check"
        );
    }
}

#[test]
fn rp2040_validation_rejects_bad_pads_channels_and_the_temperature_input() {
    let original = load("rp2040");
    for bad_pad in ["GP30x", "P26", "GPIO26", "GP", "GP26.0", "gp_26"] {
        let mut desc = original.clone();
        let mapping = desc.analog_pins.remove("GP26").unwrap();
        desc.analog_pins.insert(bad_pad.to_string(), mapping);
        assert!(
            !analog_pin_problems("bad", &desc).is_empty(),
            "accepted {bad_pad}"
        );
    }
    // Inside the label grammar, off the datasheet's pad/channel pairing: ADC0
    // is GPIO26, so naming GPIO27 on channel 0 would route a solved node to a
    // different pin than the firmware converts.
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GP26").unwrap().channel = 1;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // Input 4 is the temperature sensor, gated by `CS.TS_EN`, with no pad.
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GP29").unwrap().channel = 4;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // The converter must still be an ADC.
    let mut desc = original.clone();
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "adc")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

#[test]
fn esp32_validation_rejects_bad_pads_channels_and_types() {
    let original = load("esp32");
    assert!(analog_pin_problems("esp32", &original).is_empty());
    for bad_pad in ["GPIO034", "GPIO+34", "IO34", "GPIO", "P34", "GPIO34x"] {
        let mut desc = original.clone();
        let mapping = desc.analog_pins.remove("GPIO34").unwrap();
        desc.analog_pins.insert(bad_pad.to_string(), mapping);
        assert!(
            !analog_pin_problems("bad", &desc).is_empty(),
            "accepted {bad_pad}"
        );
    }
    // A swap keeps every channel claimed once but routes GPIO34 into CH7.
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GPIO34").unwrap().channel = 7;
    desc.analog_pins.get_mut("GPIO35").unwrap().channel = 6;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // SAR ADC1 has eight inputs; channel 8 names no pad.
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GPIO35").unwrap().channel = 8;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original.clone();
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "sens_sar_adc")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

#[test]
fn esp32c6_validation_rejects_bad_pads_channels_and_types() {
    // The C6 `apb_saradc` is the C3 model, so the C3 rule (channel == GPIO,
    // five inputs) applies until the model's channel count is per chip.
    for chip in ["esp32c6", "esp32c6fh4"] {
        let original = load(chip);
        assert!(analog_pin_problems(chip, &original).is_empty());
        let expected: BTreeMap<String, AdcPinFn> = (0..5u8)
            .map(|channel| {
                (
                    format!("GPIO{channel}"),
                    AdcPinFn {
                        peripheral: "apb_saradc".into(),
                        channel,
                    },
                )
            })
            .collect();
        assert_eq!(original.analog_pins, expected, "{chip}");
        let mut desc = original.clone();
        desc.analog_pins.get_mut("GPIO2").unwrap().channel = 3;
        desc.analog_pins.get_mut("GPIO3").unwrap().channel = 2;
        assert!(!analog_pin_problems("bad", &desc).is_empty());
        // GPIO5 is ADC1_CH5 on silicon, but the model refuses channel 5.
        let mut desc = original.clone();
        desc.analog_pins.insert(
            "GPIO5".into(),
            AdcPinFn {
                peripheral: "apb_saradc".into(),
                channel: 5,
            },
        );
        assert!(!analog_pin_problems("bad", &desc).is_empty());
    }
}

#[test]
fn esp32c5_validation_rejects_bad_pads_channels_and_types() {
    // ESP-IDF esp32c5 adc_channel.h: ADC1_CH0..CH5 are GPIO1..GPIO6. The
    // model publishes CH0..CH4.
    let original = load("esp32c5");
    assert!(analog_pin_problems("esp32c5", &original).is_empty());
    let expected: BTreeMap<String, AdcPinFn> = (0u8..5)
        .map(|channel| {
            (
                format!("GPIO{}", channel + 1),
                AdcPinFn {
                    peripheral: "apb_saradc".into(),
                    channel,
                },
            )
        })
        .collect();
    assert_eq!(original.analog_pins, expected);
    // The C3/C6 rule (channel == GPIO) is wrong for the C5.
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GPIO3").unwrap().channel = 3;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // GPIO0 has no ADC input on the C5, whatever channel it claims.
    let mut desc = original.clone();
    let mapping = desc.analog_pins.remove("GPIO1").unwrap();
    desc.analog_pins.insert("GPIO0".into(), mapping);
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // The model has five C5 ADC1 inputs; GPIO7 has none at all.
    let mut desc = original.clone();
    desc.analog_pins.insert(
        "GPIO7".into(),
        AdcPinFn {
            peripheral: "apb_saradc".into(),
            channel: 5,
        },
    );
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

#[test]
fn esp32c61_validation_rejects_bad_pads_channels_and_types() {
    // ESP-IDF esp32c61 adc_channel.h: ADC1_CH0..CH3 are GPIO1, 3, 4, 5.
    let original = load("esp32c61");
    assert!(analog_pin_problems("esp32c61", &original).is_empty());
    let expected: BTreeMap<String, AdcPinFn> = [1u8, 3, 4, 5]
        .into_iter()
        .enumerate()
        .map(|(channel, gpio)| {
            (
                format!("GPIO{gpio}"),
                AdcPinFn {
                    peripheral: "apb_saradc".into(),
                    channel: channel as u8,
                },
            )
        })
        .collect();
    assert_eq!(original.analog_pins, expected);
    // The C3/C6 rule (channel == GPIO) is wrong for the C61.
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GPIO3").unwrap().channel = 3;
    desc.analog_pins.get_mut("GPIO5").unwrap().channel = 1;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // GPIO2 has no ADC input on the C61, whatever channel it claims.
    let mut desc = original.clone();
    let mapping = desc.analog_pins.remove("GPIO1").unwrap();
    desc.analog_pins.insert("GPIO2".into(), mapping);
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // The C61 SAR ADC1 has four inputs.
    let mut desc = original.clone();
    desc.analog_pins.insert(
        "GPIO6".into(),
        AdcPinFn {
            peripheral: "apb_saradc".into(),
            channel: 4,
        },
    );
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

#[test]
fn esp32c3_validation_rejects_bad_pads_channels_and_types() {
    let original = load("esp32c3");
    assert!(analog_pin_problems("esp32c3", &original).is_empty());
    let expected: BTreeMap<String, AdcPinFn> = (0..5u8)
        .map(|channel| {
            (
                format!("GPIO{channel}"),
                AdcPinFn {
                    peripheral: "apb_saradc".into(),
                    channel,
                },
            )
        })
        .collect();
    assert_eq!(original.analog_pins, expected);
    // GPIO5 is ADC2_CH0 on silicon; any slot it takes is an ADC1 channel of
    // another pad, so the validator must refuse it.
    let mut desc = original.clone();
    let mapping = desc.analog_pins.remove("GPIO4").unwrap();
    desc.analog_pins.insert("GPIO5".into(), mapping);
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GPIO3").unwrap().channel = 4;
    desc.analog_pins.get_mut("GPIO4").unwrap().channel = 3;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GPIO4").unwrap().channel = 5;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original.clone();
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "apb_saradc")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

#[test]
fn esp32s3_validation_rejects_bad_pads_channels_and_types() {
    let original = load("esp32s3");
    assert!(analog_pin_problems("esp32s3", &original).is_empty());
    let expected: BTreeMap<String, AdcPinFn> = (0..10u8)
        .map(|channel| {
            (
                format!("GPIO{}", channel + 1),
                AdcPinFn {
                    peripheral: "sens_s3".into(),
                    channel,
                },
            )
        })
        .collect();
    assert_eq!(original.analog_pins, expected);
    // The C3 rule (channel == GPIO) is wrong on the S3: GPIO4 is ADC1_CH3.
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GPIO4").unwrap().channel = 4;
    desc.analog_pins.get_mut("GPIO5").unwrap().channel = 3;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    // GPIO11 is ADC2_CH0 on silicon; any slot it takes belongs to another pad.
    let mut desc = original.clone();
    let mapping = desc.analog_pins.remove("GPIO10").unwrap();
    desc.analog_pins.insert("GPIO11".into(), mapping);
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original.clone();
    desc.analog_pins.get_mut("GPIO10").unwrap().channel = 10;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original.clone();
    let mapping = desc.analog_pins.remove("GPIO1").unwrap();
    desc.analog_pins.insert("IO1".into(), mapping);
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original.clone();
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "sens_s3")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

#[test]
fn esp32c3_family_pad_table_rejects_an_unlisted_chip() {
    // A new chip on the C3 APB_SARADC model must add its own pad table; a
    // silent channel == GPIO fallback would accept a wrong map.
    assert_eq!(esp32c3_family_adc1_gpio("esp32h2"), None);
    assert_eq!(
        esp32c3_family_adc1_gpio("esp32c6fh4"),
        Some(&[0u8, 1, 2, 3, 4][..])
    );
}
