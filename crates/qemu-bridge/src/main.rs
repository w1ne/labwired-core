// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! LabWired side of QEMU's `labwired-mmio-bridge`.
//!
//! The board comes from its device tree: every enabled node whose
//! `compatible` is in the model registry becomes one model instance at its
//! DT address, served on its own unix socket (`<socket-dir>/<label>.sock`).
//!
//! ```text
//! qemu-bridge dt-report --dtb FILE [--json]
//! qemu-bridge plan      --dtb FILE --socket-dir DIR [--no-irq-or]
//! qemu-bridge serve     --dtb FILE --socket-dir DIR [--attach LABEL:PART@ADDR]...
//!                       [--transcript-dir DIR] [--trace] [--core-clk-hz HZ]
//! ```
//!
//! Wire protocol v2: `qemu/PROTOCOL.md`.

mod dt;
mod fdt;
mod proto;

use std::fs::File;
use std::io::BufWriter;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use labwired_core::peripherals::bcm2835_bsc::{Bcm2835Bsc, DEFAULT_CORE_CLK_HZ};
use labwired_core::peripherals::components::bme280::Bme280;
use labwired_core::peripherals::device::{I2cDevice, I2cSlaveHost};

use dt::{Board, Class, DtNode, ModelKind};
use proto::{Bridge, PROTOCOL_VERSION};

const USAGE: &str = "usage:
  qemu-bridge dt-report --dtb FILE [--json]
  qemu-bridge plan      --dtb FILE --socket-dir DIR [--no-irq-or]
  qemu-bridge serve     --dtb FILE --socket-dir DIR [--attach LABEL:PART@ADDR]...
                        [--transcript-dir DIR] [--trace] [--core-clk-hz HZ (default 350000000)]
  parts: bme280";

#[derive(Default)]
struct Args {
    dtb: Option<PathBuf>,
    socket_dir: Option<PathBuf>,
    transcript_dir: Option<PathBuf>,
    attach: Vec<String>,
    json: bool,
    no_irq_or: bool,
    trace: bool,
    core_clk_hz: Option<u64>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--dtb" => a.dtb = Some(val()?.into()),
            "--socket-dir" => a.socket_dir = Some(val()?.into()),
            "--transcript-dir" => a.transcript_dir = Some(val()?.into()),
            "--attach" => a.attach.push(val()?),
            "--json" => a.json = true,
            "--no-irq-or" => a.no_irq_or = true,
            "--trace" => a.trace = true,
            "--core-clk-hz" => {
                a.core_clk_hz = Some(
                    val()?
                        .parse()
                        .ok()
                        .filter(|&v: &u64| v > 0)
                        .ok_or("--core-clk-hz needs a positive integer")?,
                )
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(a)
}

fn load_board(a: &Args) -> Result<Board, String> {
    let path = a.dtb.as_ref().ok_or("--dtb FILE is required")?;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Board::load(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn hex(v: Option<u64>) -> String {
    v.map(|v| format!("{v:#x}")).unwrap_or_else(|| "-".into())
}

fn irq_text(n: &DtNode) -> String {
    if n.irqs.is_empty() {
        "-".into()
    } else {
        n.irqs
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn summary(board: &Board) -> String {
    let (m, total) = board.coverage();
    let pct = if total == 0 {
        0.0
    } else {
        100.0 * m as f64 / total as f64
    };
    format!("modeled {m} of {total} enabled device nodes ({pct:.1}%)")
}

fn cmd_dt_report(a: &Args) -> Result<(), String> {
    let board = load_board(a)?;
    if a.json {
        let rows: Vec<String> = board
            .nodes
            .iter()
            .map(|n| {
                let model = match n.class {
                    Class::Modeled(k) => json_str(k.name()),
                    _ => "null".into(),
                };
                format!(
                    "{{\"path\":{},\"label\":{},\"compatible\":[{}],\"cpu_addr\":{},\"size\":{},\"irqs\":[{}],\"enabled\":{},\"class\":{},\"model\":{}}}",
                    json_str(&n.path),
                    json_str(&n.label),
                    n.compatible.iter().map(|c| json_str(c)).collect::<Vec<_>>().join(","),
                    n.cpu_addr.map(|v| v.to_string()).unwrap_or("null".into()),
                    n.size.map(|v| v.to_string()).unwrap_or("null".into()),
                    n.irqs.iter().map(|i| json_str(&i.to_string())).collect::<Vec<_>>().join(","),
                    n.enabled,
                    json_str(n.class.name()),
                    model
                )
            })
            .collect();
        let (m, total) = board.coverage();
        println!(
            "{{\"nodes\":[\n{}\n],\"modeled\":{m},\"enabled_device_nodes\":{total},\"summary\":{}}}",
            rows.join(",\n"),
            json_str(&summary(&board))
        );
        return Ok(());
    }
    println!(
        "{:<54} {:<34} {:>12} {:>10} {:<16} {:<10} LABEL/MODEL",
        "PATH", "COMPATIBLE", "CPU-ADDR", "SIZE", "IRQ", "CLASS"
    );
    for n in &board.nodes {
        let model = match n.class {
            Class::Modeled(k) => format!("{} -> {}", n.label, k.name()),
            _ => n.label.clone(),
        };
        let extra = if n.reg_entries > 1 {
            format!(" (+{} reg)", n.reg_entries - 1)
        } else {
            String::new()
        };
        println!(
            "{:<54} {:<34} {:>12} {:>10} {:<16} {:<10} {}{}",
            n.path,
            n.compatible.first().map(String::as_str).unwrap_or("-"),
            hex(n.cpu_addr),
            hex(n.size),
            irq_text(n),
            n.class.name(),
            model,
            extra
        );
    }
    println!("{}", summary(&board));
    Ok(())
}

fn socket_path(dir: &Path, label: &str) -> PathBuf {
    dir.join(format!("{label}.sock"))
}

fn cmd_plan(a: &Args) -> Result<(), String> {
    let board = load_board(a)?;
    let dir = a
        .socket_dir
        .as_ref()
        .ok_or("--socket-dir DIR is required")?;
    for n in board.modeled() {
        let spi = n
            .spi()
            .ok_or_else(|| format!("{} ({}): no GIC SPI interrupt", n.label, n.path))?;
        let mut dev = format!(
            "labwired-mmio-bridge,addr={:#x},size={:#x},socket={},irq={spi}",
            n.cpu_addr.unwrap(),
            n.size.unwrap_or(0x1000),
            socket_path(dir, &n.label).display()
        );
        if a.no_irq_or {
            dev.push_str(",irq-or=off");
        }
        println!("-device {dev}");
    }
    Ok(())
}

struct Attach {
    label: String,
    part: Box<dyn I2cDevice>,
    desc: String,
}

fn parse_attach(spec: &str) -> Result<Attach, String> {
    let err = || format!("--attach {spec}: expected LABEL:PART@ADDR, e.g. i2c1:bme280@0x76");
    let (label, rest) = spec.split_once(':').ok_or_else(err)?;
    let (part, addr) = rest.split_once('@').ok_or_else(err)?;
    let addr = addr.trim_start_matches("0x");
    let addr = u8::from_str_radix(addr, 16).map_err(|_| err())?;
    let part: Box<dyn I2cDevice> = match part {
        "bme280" => Box::new(Bme280::new(addr)),
        p => {
            return Err(format!(
                "--attach {spec}: unknown part {p:?} (known: bme280)"
            ))
        }
    };
    Ok(Attach {
        label: label.into(),
        part,
        desc: rest.into(),
    })
}

fn cmd_serve(a: &Args) -> Result<(), String> {
    let board = load_board(a)?;
    let dir = a
        .socket_dir
        .as_ref()
        .ok_or("--socket-dir DIR is required")?;
    let clk = a.core_clk_hz.unwrap_or(DEFAULT_CORE_CLK_HZ);
    let mut instances: Vec<(String, Bridge)> = Vec::new();
    for n in board.modeled() {
        let bridge = match n.class {
            Class::Modeled(ModelKind::Bcm2835Bsc) => Bridge::new(Bcm2835Bsc::new(clk)),
            _ => unreachable!(),
        };
        instances.push((n.label.clone(), bridge));
    }
    for spec in &a.attach {
        let at = parse_attach(spec)?;
        let Some((_, bridge)) = instances.iter_mut().find(|(l, _)| *l == at.label) else {
            let why = match board.by_label(&at.label) {
                Some(n) => format!("{} is {} in the DTB", at.label, n.class.name()),
                None => format!("no node labelled {} in the DTB", at.label),
            };
            return Err(format!("--attach {spec}: {why}"));
        };
        bridge.bsc.attach_slave(at.part);
        eprintln!("[qemu-bridge] {}: attached {}", at.label, at.desc);
    }
    if instances.is_empty() {
        return Err("the DTB has no enabled node the registry can model".into());
    }
    eprintln!(
        "[qemu-bridge] protocol v{PROTOCOL_VERSION}, core clock {clk} Hz, {}",
        summary(&board)
    );
    let mut handles = Vec::new();
    for (label, mut bridge) in instances {
        let n = board.by_label(&label).unwrap();
        bridge.label = label.clone();
        bridge.trace = a.trace;
        if let Some(td) = &a.transcript_dir {
            let p = td.join(format!("{label}.transcript"));
            let f = File::create(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            bridge.transcript = Some(Box::new(BufWriter::new(f)));
        }
        let sock = socket_path(dir, &label);
        let _ = std::fs::remove_file(&sock);
        let listener =
            UnixListener::bind(&sock).map_err(|e| format!("bind {}: {e}", sock.display()))?;
        eprintln!(
            "[qemu-bridge] {label}: {} at {:#x} SPI {:?} on {}",
            n.path,
            n.cpu_addr.unwrap(),
            n.spi(),
            sock.display()
        );
        handles.push(std::thread::spawn(move || -> Result<(), String> {
            let (stream, _) = listener.accept().map_err(|e| format!("{label}: accept: {e}"))?;
            let res = proto::serve(stream, &mut bridge);
            if let Some(w) = bridge.transcript.as_mut() {
                let _ = w.flush();
            }
            let _ = std::fs::remove_file(&sock);
            let msgs = res.map_err(|e| format!("{label}: {e}"))?;
            eprintln!(
                "[qemu-bridge] {label}: QEMU disconnected after {msgs} messages, {} IRQ raises, model time {} ns",
                bridge.irq_raises,
                bridge.bsc.now_ns()
            );
            Ok(())
        }));
    }
    let mut first_err = None;
    for h in handles {
        if let Err(e) = h
            .join()
            .unwrap_or_else(|_| Err("instance thread panicked".into()))
        {
            eprintln!("[qemu-bridge] {e}");
            first_err.get_or_insert(e);
        }
    }
    first_err.map_or(Ok(()), Err)
}

use std::io::Write as _;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match argv.split_first() {
        Some((c, r)) if !c.starts_with('-') => (c.as_str(), r),
        _ => ("serve", argv.as_slice()),
    };
    if matches!(cmd, "help" | "-h" | "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let res = parse_args(rest).and_then(|a| match cmd {
        "dt-report" => cmd_dt_report(&a),
        "plan" => cmd_plan(&a),
        "serve" => cmd_serve(&a),
        other => Err(format!("unknown command {other}\n{USAGE}")),
    });
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("qemu-bridge: {e}");
            ExitCode::FAILURE
        }
    }
}
