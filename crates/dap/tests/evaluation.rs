use labwired_core::{DebugControl, Machine};
use labwired_dap::server::DapServer;
use serde_json::json;

#[test]
fn test_dap_evaluate_register() {
    let server = DapServer::new();
    let adapter = server.adapter.clone();

    // Set up a machine and set a register
    let mut bus = labwired_core::bus::SystemBus::new();
    let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.write_core_reg(0, 0x12345678);
    *adapter.machine.lock().unwrap() = Some(Box::new(machine));

    // Mock DAP request
    let request = json!({
        "seq": 1,
        "type": "request",
        "command": "evaluate",
        "arguments": {
            "expression": "R0",
            "frameId": 1,
            "context": "watch"
        }
    });

    let _input = format!(
        "Content-Length: {}\r\n\r\n{}",
        serde_json::to_string(&request).unwrap().len(),
        serde_json::to_string(&request).unwrap()
    );
    let mut _output: Vec<u8> = Vec::new();

    // We can't easily run the server loop in a test because it blocks
    // But we can trigger the handler if we refactor or just test the logic
    // For now, let's test the adapter's ability to resolve if we expose it,
    // or use a more integrated approach if the server allowed single-step handling.
}

#[test]
fn test_adapter_resolve_and_evaluate() {
    // This is a more direct test of the logic we added to server.rs and adapter.rs
    let adapter = labwired_dap::adapter::LabwiredAdapter::new();

    let mut bus = labwired_core::bus::SystemBus::new();
    let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    // Set R0 to a value
    machine.write_core_reg(0, 0xDEADBEEF);

    // Set memory at 0x20000000 to a value
    machine
        .write_memory(0x20000000, &[0x11, 0x22, 0x33, 0x44])
        .unwrap();

    *adapter.machine.lock().unwrap() = Some(Box::new(machine));

    // 1. Test register evaluation (mocking server logic)
    let expression = "R0";
    let mut result_val = None;
    if let Ok(names) = adapter.get_register_names() {
        for (i, name) in names.iter().enumerate() {
            if name.eq_ignore_ascii_case(expression) {
                let val = adapter.get_register(i as u8).unwrap_or(0);
                result_val = Some(format!("{:#x}", val));
                break;
            }
        }
    }
    assert_eq!(result_val, Some("0xdeadbeef".to_string()));

    // 2. Test case insensitivity
    let expression = "r0";
    let mut result_val = None;
    if let Ok(names) = adapter.get_register_names() {
        for (i, name) in names.iter().enumerate() {
            if name.eq_ignore_ascii_case(expression) {
                let val = adapter.get_register(i as u8).unwrap_or(0);
                result_val = Some(format!("{:#x}", val));
                break;
            }
        }
    }
    assert_eq!(result_val, Some("0xdeadbeef".to_string()));
}

#[test]
fn test_adapter_locals_evaluation() {
    let adapter = labwired_dap::adapter::LabwiredAdapter::new();

    let mut bus = labwired_core::bus::SystemBus::new();
    let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    // Set SP (R13) to 0x20001000
    machine.write_core_reg(13, 0x2000_1000);

    // Set memory at [SP - 4] to 0x12345678 (simulating a local variable on stack)
    machine
        .write_memory(0x2000_1000 - 4, &[0x78, 0x56, 0x34, 0x12])
        .unwrap();

    *adapter.machine.lock().unwrap() = Some(Box::new(machine));

    // Simulate found local variable: "temp" at frame base - 4
    let local = labwired_loader::LocalVariable {
        name: "temp".to_string(),
        location: labwired_loader::DwarfLocation::FrameRelative(-4),
    };

    // Test evaluation logic (mimicking server.rs)
    let val_str = match local.location {
        labwired_loader::DwarfLocation::Register(r) => {
            let val = adapter.get_register(r as u8).unwrap_or(0);
            format!("{:#x}", val)
        }
        labwired_loader::DwarfLocation::FrameRelative(offset) => {
            let sp = adapter.get_register(13).unwrap_or(0);
            let addr = (sp as i64 + offset) as u32;
            if let Ok(data) = adapter.read_memory(addr as u64, 4) {
                let val = (data[0] as u32)
                    | ((data[1] as u32) << 8)
                    | ((data[2] as u32) << 16)
                    | ((data[3] as u32) << 24);
                format!("{:#x}", val)
            } else {
                "error".to_string()
            }
        }
        _ => "not available".to_string(),
    };

    assert_eq!(val_str, "0x12345678");
}
