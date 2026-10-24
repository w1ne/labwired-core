// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

use labwired_core::inspect::MachineInspect;

/// Bound the serialized payload across both inspect walks. Never truncate an
/// artifact: a partial framebuffer would no longer describe the actual image.
pub(crate) fn bound_inspect_bytes(inspect: &mut MachineInspect, max_bytes: usize) {
    let mut remaining = max_bytes;
    for artifact in inspect
        .peripherals
        .iter_mut()
        .flat_map(|p| &mut p.artifacts)
        .chain(inspect.devices.iter_mut().flat_map(|d| &mut d.artifacts))
    {
        let Some(bytes) = artifact.bytes.as_ref() else {
            continue;
        };
        let len = bytes.len();
        if len <= remaining {
            remaining -= len;
            continue;
        }
        artifact.bytes = None;
        if !artifact.meta.is_object() {
            let original = artifact.meta.take();
            artifact.meta = serde_json::json!({ "original_meta": original });
        }
        artifact.meta["byte_length"] = serde_json::json!(len);
        artifact.meta["bytes_omitted_reason"] = serde_json::json!("inspect_bytes_limit");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_core::inspect::{InspectOpts, PeripheralInspect};
    use labwired_core::peripherals::{components::ssd1306, i2c::I2cDevice};

    fn painted_oled() -> MachineInspect {
        let mut oled = ssd1306(0x3c);
        oled.start();
        oled.write(0x40);
        for byte in [0x81, 0x42, 0x24, 0x18] {
            oled.write(byte);
        }
        oled.stop();
        MachineInspect {
            peripherals: vec![PeripheralInspect {
                name: "i2c1".into(),
                kind: "i2c".into(),
                base: 0,
                registers: vec![],
                artifacts: I2cDevice::artifacts(
                    &oled,
                    "oled",
                    &InspectOpts {
                        include_bytes: true,
                        peripheral: None,
                    },
                ),
            }],
            devices: vec![],
        }
    }

    #[test]
    fn serialized_bytes_match_real_ssd1306_wire_writes() {
        let mut inspect = painted_oled();
        bound_inspect_bytes(&mut inspect, 1024);
        let json = serde_json::to_value(inspect).unwrap();
        let bytes: Vec<u8> =
            serde_json::from_value(json["peripherals"][0]["artifacts"][0]["bytes"].clone())
                .unwrap();
        assert_eq!(bytes.len(), 1024);
        assert_eq!(&bytes[..4], &[0x81, 0x42, 0x24, 0x18]);
        assert!(bytes[4..].iter().all(|&b| b == 0));
    }

    #[test]
    fn budget_is_cumulative_across_complete_artifacts() {
        let mut inspect = painted_oled();
        let second = inspect.peripherals[0].artifacts[0].clone();
        inspect.peripherals[0].artifacts.push(second);
        bound_inspect_bytes(&mut inspect, 1536);
        let artifacts = &inspect.peripherals[0].artifacts;
        assert_eq!(artifacts[0].bytes.as_ref().unwrap().len(), 1024);
        assert!(artifacts[1].bytes.is_none());
        assert_eq!(artifacts[1].meta["byte_length"], 1024);
        assert_eq!(
            artifacts[1].meta["bytes_omitted_reason"],
            "inspect_bytes_limit"
        );
    }
}
