//! Runtime-edited descriptors must release their storage when their owner drops.
//! This binary deliberately has one test: allocator accounting is thread-local,
//! and every measured operation constructs and drops on the test thread.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use labwired_core::peripherals::components::{
    declarative_analog::DeclarativeAnalogKit, declarative_gpio::DeclarativeGpioKit,
    declarative_led_strip::DeclarativeLedStripKit, declarative_uart::DeclarativeUartKit,
    DeclarativeDisplayKit, DeclarativeI2cKit, DeclarativeSpiKit, GenericI2cDevice,
    GenericSpiDevice,
};
use labwired_core::peripherals::kit::{declarative::DeclarativeDeviceKit, PeripheralKit};

struct CountedSystem;
thread_local! { static LIVE_BYTES: Cell<isize> = const { Cell::new(0) }; }

unsafe impl GlobalAlloc for CountedSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let _ = LIVE_BYTES.try_with(|bytes| bytes.set(bytes.get() + layout.size() as isize));
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let _ = LIVE_BYTES.try_with(|bytes| bytes.set(bytes.get() - layout.size() as isize));
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: CountedSystem = CountedSystem;

fn live_bytes() -> isize {
    LIVE_BYTES.with(Cell::get)
}

fn exercise(device_type: &str, edit: usize) {
    let mut descriptor = labwired_config::DeviceDescriptor::from_yaml(
        labwired_config::embedded_device_yaml(device_type).unwrap(),
    )
    .unwrap();
    descriptor.r#type = format!("private-{device_type}-{edit}");
    let yaml = serde_yaml::to_string(&descriptor).unwrap();
    let kit: Box<dyn PeripheralKit> = match descriptor.behavior.primitive.as_str() {
        "i2c_device" => {
            // Direct construction is a separate route from kits and leaked a
            // fresh discovery table for every instance of even the same YAML.
            drop(GenericI2cDevice::from_yaml(&yaml, 0).unwrap());
            Box::new(DeclarativeI2cKit::from_yaml(&yaml).unwrap())
        }
        "spi_device" => {
            drop(GenericSpiDevice::from_yaml(&yaml, "GPIO7").unwrap());
            Box::new(DeclarativeSpiKit::from_yaml(&yaml).unwrap())
        }
        "analog_source" => Box::new(DeclarativeAnalogKit::from_yaml(&yaml).unwrap()),
        "display" => Box::new(DeclarativeDisplayKit::from_yaml(&yaml).unwrap()),
        "gpio_device" => Box::new(DeclarativeGpioKit::from_yaml(&yaml).unwrap()),
        "uart_device" => Box::new(DeclarativeUartKit::from_yaml(&yaml).unwrap()),
        "led_strip" => Box::new(DeclarativeLedStripKit::from_yaml(&yaml).unwrap()),
        _ => Box::new(DeclarativeDeviceKit::from_yaml(&yaml).unwrap()),
    };
    assert_eq!(kit.metadata().device_type, descriptor.r#type);
    assert!(!serde_json::to_string(kit.metadata()).unwrap().is_empty());
}

fn exercise_bus(device_type: &str, edit: usize, fail_attach: bool) {
    let mut pack = labwired_config::DeviceDescriptor::from_yaml(
        labwired_config::embedded_device_yaml(device_type).unwrap(),
    )
    .unwrap();
    // Reload edited definitions under the same public type. A type-only cache
    // must not serve the previous definition's discovery metadata.
    pack.r#type = format!("private-{device_type}-{fail_attach}");
    pack.metadata.as_mut().unwrap().inputs[0].label = format!("edited channel {edit}");
    pack.schema = Some("labwired.part/v1".into());
    let (connection, config) = match pack.behavior.primitive.as_str() {
        "spi_device" => ("spi2", serde_json::json!({"cs_pin": "GPIO7"})),
        "analog_source" => ("apb_saradc", serde_json::json!({"channel": 3})),
        _ => ("i2c0", serde_json::json!({})),
    };
    let mut root: serde_yaml::Mapping = serde_yaml::from_str(&format!(
        "schema_version: '1.0'\nname: owned-runtime-pack\nchip: esp32c3\nexternal_devices:\n  - id: sensor\n    type: {}\n    connection: {}\n    route: {{sda: GPIO4, scl: GPIO5}}\n    config: {}\n",
        pack.r#type,
        if fail_attach { "missing-controller" } else { connection },
        config,
    )).unwrap();
    root.insert(
        "parts".into(),
        serde_yaml::Value::Sequence(vec![serde_yaml::to_value(&pack).unwrap()]),
    );
    let manifest =
        labwired_config::SystemManifest::from_yaml(&serde_yaml::to_string(&root).unwrap()).unwrap();
    let chip = labwired_config::ChipDescriptor::from_file(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../configs/chips/esp32c3.yaml"),
    )
    .unwrap();
    let result = labwired_core::bus::SystemBus::from_config(&chip, &manifest);
    if fail_attach {
        assert!(result.is_err(), "invalid controller must reject attachment");
    } else {
        let mut bus = result.unwrap();
        let discovery = bus.list_inputs();
        let (_, channel) = discovery
            .iter()
            .find(|(owner, _)| owner == "sensor")
            .unwrap();
        assert_eq!(channel.label, format!("edited channel {edit}"));
        bus.set_input(Some("sensor"), channel.key.as_ref(), channel.min)
            .unwrap();
        drop(bus);
        // Discovery is an owned snapshot: its strings survive device teardown.
        assert!(!channel.key.is_empty());
        assert!(!serde_json::to_string(&discovery).unwrap().is_empty());
    }
}

#[test]
fn edited_runtime_descriptors_release_all_owned_allocations() {
    let mut failures = Vec::new();
    // Exercise all descriptor families, including generic pin timing metadata.
    for device_type in [
        "tmp102",
        "veml7700",
        "adxl345_spi",
        "gp2y0a21",
        "oled-ssd1306",
        "hx711",
        "neo6m-gps",
        "ws2812",
        "keypad",
        "dht22",
        "rotary_encoder",
        "hc-sr04",
    ] {
        exercise(device_type, 0); // Warm any one-time runtime initialization.
        let before = live_bytes();
        for edit in 1..17 {
            exercise(device_type, edit);
        }
        let retained = live_bytes() - before;
        if retained != 0 {
            failures.push(format!(
                "{device_type} constructors retained {retained} bytes after 16 edits"
            ));
        }
    }
    for device_type in ["veml7700", "adxl345_spi", "gp2y0a21"] {
        for fail_attach in [false, true] {
            if fail_attach && device_type != "adxl345_spi" {
                // I2C/ADC accept fallback controllers; SPI rejects a missing
                // controller and exercises fallible runtime-kit attachment.
                continue;
            }
            exercise_bus(device_type, 0, fail_attach);
            let before = live_bytes();
            for edit in 1..9 {
                exercise_bus(device_type, edit, fail_attach);
            }
            let retained = live_bytes() - before;
            if retained != 0 {
                failures.push(format!("{device_type} buses (failed={fail_attach}) retained {retained} bytes after 8 edits"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
