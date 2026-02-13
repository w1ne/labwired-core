use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[test]
fn test_dap_e2e_launch_and_uart() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let firmware_path = root.join("../../target/thumbv7m-none-eabi/debug/firmware-ci-fixture");

    if !firmware_path.exists() {
        return;
    }

    // Start labwired-dap
    let mut child = Command::new("cargo")
        .args(["run", "-p", "labwired-dap", "-q"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to start labwired-dap");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    // 1. Send Initialize
    let mut uart_buffer = String::new();
    let init_req =
        r#"{"seq":1,"type":"request","command":"initialize","arguments":{"adapterID":"labwired"}}"#;
    send_dap_message(&mut stdin, init_req);

    // Read response
    let resp = read_dap_message(&mut reader);
    assert!(resp.contains("\"command\":\"initialize\""));

    // 2. Send Launch
    let launch_req = format!(
        r#"{{"seq":2,"type":"request","command":"launch","arguments":{{"program":"{}"}}}}"#,
        firmware_path.to_str().unwrap().replace('\\', "/")
    );
    send_dap_message(&mut stdin, &launch_req);

    // Read response
    let resp = read_dap_message(&mut reader);
    assert!(resp.contains("\"command\":\"launch\""));

    // 3. Send ConfigurationDone
    let config_done = r#"{"seq":3,"type":"request","command":"configurationDone"}"#;
    send_dap_message(&mut stdin, config_done);
    let resp = wait_for_response(&mut reader, "configurationDone", &mut uart_buffer);
    assert!(resp.contains("\"command\":\"configurationDone\""));

    // 4. Send Continue
    let continue_req =
        r#"{"seq":4,"type":"request","command":"continue","arguments":{"threadId":1}}"#;
    send_dap_message(&mut stdin, continue_req);
    let resp = wait_for_response(&mut reader, "continue", &mut uart_buffer);
    assert!(resp.contains("\"command\":\"continue\""));

    // 5. Wait for UART output event if not already received
    if !uart_buffer.contains("OK") {
        eprintln!("Waiting for UART output event...");
        for _ in 0..50 {
            let event = read_dap_message(&mut reader);
            eprintln!("Received: {}", event);
            if event.contains("\"event\":\"output\"") && event.contains("OK") {
                uart_buffer.push_str(&event);
                break;
            }
        }
    }

    assert!(
        uart_buffer.contains("OK"),
        "Did not receive UART output event containing 'OK'"
    );

    // Cleanup
    let _ = child.kill();
    let _ = child.wait();
    eprintln!("E2E Test Passed!");
}

fn wait_for_response(
    reader: &mut BufReader<std::process::ChildStdout>,
    command: &str,
    uart_buffer: &mut String,
) -> String {
    for _ in 0..100 {
        let msg = read_dap_message(reader);
        eprintln!("Received: {}", msg);
        if msg.contains("\"type\":\"response\"")
            && msg.contains(&format!("\"command\":\"{}\"", command))
        {
            return msg;
        }
        if msg.contains("\"event\":\"output\"") {
            uart_buffer.push_str(&msg);
        }
    }
    panic!("Timed out waiting for response to {}", command);
}

fn send_dap_message(stdin: &mut dyn Write, message: &str) {
    eprintln!("Sending: {}", message);
    write!(
        stdin,
        "Content-Length: {}\r\n\r\n{}",
        message.len(),
        message
    )
    .unwrap();
    stdin.flush().unwrap();
}

fn read_dap_message(reader: &mut BufReader<std::process::ChildStdout>) -> String {
    let mut line = String::new();

    // 1. Find Content-Length header
    let content_length = loop {
        line.clear();
        if reader.read_line(&mut line).unwrap() == 0 {
            panic!("Unexpected EOF while reading DAP message");
        }
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Content-Length: ") {
            let parsed: usize = rest.trim().parse().expect("Invalid Content-Length");
            break parsed;
        }
    };

    // 2. Read until the double newline (end of headers)
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if line.trim().is_empty() {
            break;
        }
    }

    // 3. Read the body
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).unwrap();
    String::from_utf8(body).unwrap()
}
