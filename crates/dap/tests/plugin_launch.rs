use labwired_config::PeripheralConfig;
use labwired_core::plugin::{ChipPlugin, PeripheralBuildCtx, PLUGIN_API_VERSION};
use labwired_core::{Peripheral, SimResult};
use labwired_dap::adapter::LabwiredAdapter;
use labwired_dap::server::DapServer;
use serde_json::{json, Value};
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

const CHIP_NAME: &str = "test-dap-plugin-chip";
const GPIO_OUTPUT_OFFSET: u64 = 0x2200;
const CHIP_YAML: &str = r#"
name: "test-dap-plugin-chip"
arch: "arm"
core: "cortex-m0+"
flash: { base: 0x0, size: "64KB" }
ram: { base: 0x20000000, size: "4KB" }
peripherals:
  - id: plugin0
    type: test_dap_plugin_peripheral
    base_address: 0x40000000
    size: 16KB
    config:
      gpio_offsets: { input: 0x2100, output: 0x2200 }
"#;

#[derive(Debug)]
struct TestPeripheral;

impl Peripheral for TestPeripheral {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(if offset == GPIO_OUTPUT_OFFSET {
            0x5A
        } else {
            0
        })
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(if offset == GPIO_OUTPUT_OFFSET {
            0x5A
        } else {
            0
        })
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }
}

struct TestPlugin;

impl ChipPlugin for TestPlugin {
    fn api_version(&self) -> u32 {
        PLUGIN_API_VERSION
    }

    fn chip_names(&self) -> &[&str] {
        &[CHIP_NAME]
    }

    fn chip_yaml(&self, name: &str) -> Option<&'static str> {
        (name == CHIP_NAME).then_some(CHIP_YAML)
    }

    fn try_build_peripheral(
        &self,
        ctx: &PeripheralBuildCtx<'_>,
        _config: &PeripheralConfig,
    ) -> Option<anyhow::Result<Box<dyn Peripheral>>> {
        (ctx.canonical_type == "test_dap_plugin_peripheral")
            .then(|| Ok(Box::new(TestPeripheral) as Box<dyn Peripheral>))
    }
}

#[derive(Clone)]
struct SharedOutput(Arc<Mutex<Vec<u8>>>);

impl Write for SharedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn frame(seq: u64, command: &str, arguments: Option<Value>) -> String {
    let mut message = json!({ "seq": seq, "type": "request", "command": command });
    if let Some(arguments) = arguments {
        message["arguments"] = arguments;
    }
    let body = message.to_string();
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
}

fn parse_messages(bytes: &[u8]) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let header_end = bytes[offset..]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| offset + index)
            .expect("DAP header terminator");
        let headers = std::str::from_utf8(&bytes[offset..header_end]).expect("header UTF-8");
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .expect("Content-Length header")
            .parse::<usize>()
            .expect("numeric Content-Length");
        let body_start = header_end + 4;
        let body_end = body_start + content_length;
        messages.push(serde_json::from_slice(&bytes[body_start..body_end]).expect("DAP JSON"));
        offset = body_end;
    }
    messages
}

#[test]
fn dap_launch_resolves_and_builds_a_registered_plugin_chip() {
    let system_path = std::env::temp_dir().join(format!(
        "labwired-dap-plugin-system-{}.yaml",
        std::process::id()
    ));
    std::fs::write(
        &system_path,
        format!(
            "name: plugin test board\nchip: {CHIP_NAME}\nboard_io:\n  - id: plugin-led\n    kind: led\n    peripheral: plugin0\n    pin: 1\n    signal: output\n"
        ),
    )
    .expect("write temporary system manifest");
    let firmware_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/uart-ok-thumbv7m.elf");
    assert!(firmware_path.exists(), "committed test ELF is present");

    let server = DapServer::with_plugins(vec![Arc::new(TestPlugin)]);
    let mut input = frame(1, "initialize", Some(json!({ "adapterID": "labwired" })));
    input.push_str(&frame(
        2,
        "launch",
        Some(json!({
            "program": firmware_path,
            "systemConfig": system_path,
            "stopOnEntry": true
        })),
    ));
    input.push_str(&frame(3, "disconnect", None));

    let output = Arc::new(Mutex::new(Vec::new()));
    server
        .run(
            Cursor::new(input.into_bytes()),
            SharedOutput(output.clone()),
        )
        .expect("DAP server handles plugin launch");
    let board_io = server
        .adapter
        .get_telemetry()
        .expect("telemetry exists after plugin launch")
        .board_io;
    let output = output.lock().unwrap().clone();
    let launch = parse_messages(&output)
        .into_iter()
        .find(|message| message["command"] == "launch")
        .expect("launch response is emitted");

    let _ = std::fs::remove_file(system_path);
    assert_eq!(launch["success"], true, "launch response: {launch}");
    assert_eq!(board_io.len(), 1);
    assert_eq!(board_io[0].id, "plugin-led");
    assert_eq!(board_io[0].kind, "led");
    assert_eq!(board_io[0].signal, "output");
    assert!(
        board_io[0].active,
        "DAP reads the configured GPIO output register at {GPIO_OUTPUT_OFFSET:#x}"
    );
}

#[test]
fn arduino_esp32_adapter_path_attaches_registered_plugin_peripherals() {
    let system_path = std::env::temp_dir().join(format!(
        "labwired-dap-arduino-plugin-system-{}.yaml",
        std::process::id()
    ));
    std::fs::write(
        &system_path,
        r#"
name: Arduino plugin board
chip: esp32
peripherals:
  - id: plugin0
    type: test_dap_plugin_peripheral
    base_address: 0x70000000
    size: 16KB
"#,
    )
    .expect("write temporary Arduino system manifest");
    let firmware_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/source-debug/esp32-arduino.elf");
    assert!(
        firmware_path.exists(),
        "committed Arduino ESP32 ELF is present"
    );

    let adapter = LabwiredAdapter::with_plugins(vec![Arc::new(TestPlugin)]);
    adapter
        .load_firmware(firmware_path, Some(system_path.clone()))
        .expect("Arduino ESP32 firmware loads with plugins");
    // 0x2200 is the plugin GPIO output register. A 4KB window ends at 0x1000,
    // so the read has to use the same 16KB window as the plugin chip YAML or
    // the address stays unmapped and DAP flattens it to 0.
    let plugin_addr = 0x7000_0000 + GPIO_OUTPUT_OFFSET;
    let (bytes, unreadable) = adapter
        .peek_memory(plugin_addr, 1)
        .expect("Arduino ESP32 plugin window is readable");

    let _ = std::fs::remove_file(system_path);
    assert_eq!(
        unreadable, 0,
        "Arduino-ESP32 builder maps the plugin peripheral at {plugin_addr:#x}"
    );
    assert_eq!(
        bytes.first().copied(),
        Some(0x5A),
        "Arduino-ESP32 builder attaches plugin-owned peripheral at {plugin_addr:#x}"
    );
}
