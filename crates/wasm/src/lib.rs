use labwired_config::{BoardIoBinding, BoardIoKind, BoardIoSignal, ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::console::{ConsoleCapture, HostConsole};

// #124 Phase 4: browser-side JIT prototype. Runs the dominant
// `0x400829cc` hot block through `js_sys::WebAssembly` instead of the
// interpreter when `jit_enabled()` has been toggled on from JS.
/// Ratchet: the wasm boundary must return errors, not `null`.
#[cfg(test)]
mod error_boundary_ratchet;
mod fidelity_surface;
mod inputs;
mod inspect;
mod install;
mod jit_browser;
#[cfg(test)]
mod playground_repro;
mod traces;
mod world;
// CortexM and XtensaLx7 are used via Box<dyn Cpu>; the concrete types are
// only constructed inside the configure_* fns and immediately boxed.
use labwired_core::decoder::arm::{decode_thumb_16, decode_thumb_32};
use labwired_core::decoder::riscv::{decode_rv32, decode_rv32c};
use labwired_core::decoder::xtensa;
use labwired_core::decoder::xtensa_length;
use labwired_core::decoder::xtensa_narrow;
use labwired_core::memory::{LinearMemory, ProgramImage};
use labwired_core::peripherals::adc::Adc;
use labwired_core::system::arch_policy::{machine_family, MachineFamily};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::Arch as CoreArch;
use labwired_core::Bus;
use labwired_core::{AdvanceRequest, Cpu, Machine};
use labwired_loader::load_elf_bytes;
use wasm_bindgen::prelude::*;

// GDB-over-WASM scaffolding (`WasmGdbConn`, `WasmGdbEventLoop`, etc.) was
// removed when `WasmSimulator` switched to `Machine<Box<dyn Cpu>>` — the
// `gdbstub::target::Target` impl in `labwired-gdbstub` is concrete per arch.
// Restore once a dyn-aware Target wrapper exists.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Per-instance state for the ESP32-classic cross-core IPI bridge that lets
/// dual-core ESP-IDF firmware run on our single-CPU sim. Real silicon routes
/// FROM_CPU_INTR0/1 through DPORT's intmatrix to a CPU internal interrupt
/// bit; we sample the mapping each step and synthesise the edge on PRO_CPU.
#[derive(Default)]
struct Esp32IpiBridge {
    from_cpu_bit0: Option<u8>,
    from_cpu_bit1: Option<u8>,
    last_from_cpu0_val: u32,
    last_from_cpu1_val: u32,
    /// Per-firmware dual-core handshake byte addresses, resolved from the
    /// firmware ELF's symbol table by `install_arduino_esp32_quirks`. The
    /// keep-alive in `step_with_esp32_aids` re-writes 0x01 to each of these
    /// every 10 000 cycles so the firmware's `.bss` zero-init can't wipe
    /// them between the install and the spin-wait check. Empty when the
    /// hardcoded reference-firmware keep-alive is in use (the old
    /// `install_esp32_arduino_quirks` path).
    handshake_bytes: Vec<u32>,
}

/// A run's console capture plus the sink for the USB-Serial-JTAG block that the
/// shared C3 ROM builder installs after the bus is handed over.
type C3FlashConsole = (ConsoleCapture, Arc<Mutex<Vec<u8>>>);

#[wasm_bindgen]
pub struct WasmSimulator {
    machine: Option<Machine<Box<dyn Cpu>>>,
    board_io: Vec<BoardIoBinding>,
    uart_sink: Arc<Mutex<Vec<u8>>>,
    /// Both of the board's consoles, one of them shown. `uart_sink` above IS
    /// this capture's heard sink — the console the board's USB socket is wired
    /// to. See [`labwired_core::console`]: the twin taps one console because a
    /// real board gives you one, and records the other so that firmware
    /// printing into a disconnected console is diagnosable instead of silent.
    console: ConsoleCapture,
    uart_rx_bufs: Vec<Arc<Mutex<VecDeque<u8>>>>,
    #[allow(dead_code)]
    /// Which decoder the Trace panel uses. Typed as `MachineFamily`, not
    /// `Arch`, so `Unknown` is unrepresentable: the disassembler used to
    /// fold it in with Arm and print Thumb for an architecture nobody had
    /// established — the same guess that let an unknown chip boot as a
    /// Cortex-M.
    arch: MachineFamily,
    /// Set by `install_esp32_arduino_quirks` / `enable_esp32_dual_core_emulation`.
    /// When `Some`, `step_with_esp32_aids` runs the IPI bridge + dual-core
    /// handshake keep-alives each cycle.
    esp32_ipi: Option<Esp32IpiBridge>,
    /// #124 Phase 4: browser-side JIT cache. Off by default — flip via
    /// `set_jit_enabled(true)` from JS. We deliberately don't auto-enable
    /// until benchmarks confirm a net win, so production playground
    /// behaviour is unchanged unless the operator opts in.
    jit_browser_enabled: bool,
    /// Lazy-init at first JIT-able step. Boxed so the typical "JIT off"
    /// path pays no per-instance allocation.
    jit_browser_cache: Option<Box<jit_browser::BrowserJitCache>>,
}

/// Inject the JSON body the virtual WiFi AP serves for
/// `GET /v1/public-stats` (LBC3.1 stats lab). The browser playground should
/// `fetch('https://api.labwired.com/v1/public-stats')` and pass the text here
/// **before** constructing the simulator so the device twin receives live
/// product numbers. Pass an empty string to clear the override (baked
/// fallback). Wasm has no sockets; native CLI fetches live itself.
#[wasm_bindgen]
pub fn set_wifi_ap_public_stats_json(json: &str) {
    if json.is_empty() {
        labwired_core::peripherals::esp32c3::virtual_wifi::set_public_stats_body(None);
    } else {
        labwired_core::peripherals::esp32c3::virtual_wifi::set_public_stats_body(Some(
            json.as_bytes().to_vec(),
        ));
    }
}

/// Enable browser host-network bridge so the virtual AP grants stations
/// internet via JS (DoH + `fetch`). Call once after loading the wasm module.
#[wasm_bindgen]
pub fn wifi_host_net_set_active(active: bool) {
    labwired_core::peripherals::esp32c3::virtual_wifi_host_net::set_bridge_active(active);
}

/// Pending DNS names the host must resolve (DoH). JSON array of
/// `{ "id": number, "name": string }`.
#[wasm_bindgen]
pub fn wifi_host_poll_dns_requests() -> String {
    let reqs = labwired_core::peripherals::esp32c3::virtual_wifi_host_net::poll_dns_requests();
    let v: Vec<serde_json::Value> = reqs
        .into_iter()
        .map(|r| serde_json::json!({ "id": r.id, "name": r.name }))
        .collect();
    serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
}

/// Fulfill a DNS request with A records. `ips_json` is a JSON array of
/// dotted-quads, e.g. `["93.184.216.34"]`.
#[wasm_bindgen]
pub fn wifi_host_fulfill_dns(id: u32, ips_json: &str) {
    let ips: Vec<[u8; 4]> = serde_json::from_str::<Vec<String>>(ips_json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let parts: Vec<u8> = s.split('.').filter_map(|p| p.parse().ok()).collect();
            (parts.len() == 4).then(|| [parts[0], parts[1], parts[2], parts[3]])
        })
        .collect();
    labwired_core::peripherals::esp32c3::virtual_wifi_host_net::fulfill_dns(id, ips);
}

/// Pending HTTP proxy requests. JSON array of
/// `{ "id", "url", "method", "body_b64" }` — any host URL; body is the
/// request entity after headers (client-side `fetch` uses the user's network).
#[wasm_bindgen]
pub fn wifi_host_poll_http_requests() -> String {
    let reqs = labwired_core::peripherals::esp32c3::virtual_wifi_host_net::poll_http_requests();
    let v: Vec<serde_json::Value> = reqs
        .into_iter()
        .map(|r| {
            let body = r
                .raw_request
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|i| r.raw_request[i + 4..].to_vec())
                .unwrap_or_default();
            serde_json::json!({
                "id": r.id,
                "url": r.url,
                "method": r.method,
                "body_b64": b64_encode(&body),
            })
        })
        .collect();
    serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
}

fn b64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Fulfill an HTTP proxy request with a raw HTTP/1.1 response body (status
/// line + headers + body), as UTF-8 or binary string via byte array from JS.
#[wasm_bindgen]
pub fn wifi_host_fulfill_http(id: u32, response: &[u8]) {
    labwired_core::peripherals::esp32c3::virtual_wifi_host_net::fulfill_http(id, response.to_vec());
}

/// Public shape returned by `step_batch_profile`.
///
/// The six execution counters intentionally mirror `StepProfile` exactly.
/// `executed_cycles` is the batch boundary observable; workload-specific
/// markers such as ESP32-S3 OLED first-paint and completion remain outside
/// this generic API and are measured by the workload harness in the same
/// simulation pass.
#[derive(serde::Serialize)]
struct WasmStepBatchProfile {
    requested_cycles: u32,
    executed_cycles: u32,
    wall_ms: f64,
    cycles_per_second: f64,
    cpu_instructions: u64,
    cpu_batches: u64,
    peripheral_ticks: u64,
    peripheral_ticked_entries: u64,
    bus_tick_entries: u64,
    legacy_tick_entries: u64,
}

#[cfg(test)]
const ESP32C3_APP_IMAGE_OFFSET: usize = 0x1_0000;
const ESP_IMAGE_HEADER_LEN: usize = 24;
const ESP_IMAGE_MAGIC: u8 = 0xE9;
const ESP32C3_FLASH_FAST_START_BLOB: &str = "labwired_esp32c3_flash_fast_start";

fn esp32c3_program_image_from_flash_offset(
    flash: &[u8],
    offset: usize,
    label: &str,
) -> Result<ProgramImage, String> {
    let image = flash.get(offset..).ok_or_else(|| {
        format!("ESP32-C3 flash image is smaller than {label} offset {offset:#x}")
    })?;
    if image.len() < ESP_IMAGE_HEADER_LEN {
        return Err(format!("ESP32-C3 {label} image header is truncated"));
    }
    if image[0] != ESP_IMAGE_MAGIC {
        return Err(format!(
            "ESP32-C3 {label} image has bad magic 0x{:02x} at flash offset {offset:#x}",
            image[0],
        ));
    }

    let segment_count = image[1] as usize;
    let entry = u32::from_le_bytes(image[4..8].try_into().unwrap()) as u64;
    let mut program = ProgramImage::new(entry, CoreArch::RiscV);
    let mut cursor = ESP_IMAGE_HEADER_LEN;

    for index in 0..segment_count {
        let header = image
            .get(cursor..cursor + 8)
            .ok_or_else(|| format!("ESP32-C3 {label} segment {index} header is truncated"))?;
        let load_addr = u32::from_le_bytes(header[0..4].try_into().unwrap()) as u64;
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        cursor += 8;
        let data = image
            .get(cursor..cursor + len)
            .ok_or_else(|| format!("ESP32-C3 {label} segment {index} data is truncated"))?;
        program.add_segment(load_addr, data.to_vec());
        cursor += len;
    }

    if program.segments.is_empty() {
        return Err(format!("ESP32-C3 {label} image has no loadable segments"));
    }

    Ok(program)
}

#[cfg(test)]
fn esp32c3_app_program_image_from_merged_flash(flash: &[u8]) -> Result<ProgramImage, String> {
    esp32c3_program_image_from_flash_offset(flash, ESP32C3_APP_IMAGE_OFFSET, "app")
}

fn esp32c3_bootloader_program_image_from_merged_flash(
    flash: &[u8],
) -> Result<ProgramImage, String> {
    esp32c3_program_image_from_flash_offset(flash, 0, "bootloader")
}

fn load_program_segments_without_reset(
    machine: &mut labwired_core::Machine<Box<dyn Cpu>>,
    program_image: &ProgramImage,
) -> Result<(), String> {
    for segment in &program_image.segments {
        if machine.bus.flash.load_from_segment(segment)
            || machine.bus.ram.load_from_segment(segment)
            || machine
                .bus
                .extra_mem
                .iter_mut()
                .any(|m| m.load_from_segment(segment))
        {
            continue;
        }

        for (i, byte) in segment.data.iter().enumerate() {
            let addr = segment.start_addr + i as u64;
            machine
                .bus
                .write_u8(addr, *byte)
                .map_err(|e| format!("load segment at {addr:#x}: {e}"))?;
        }
    }

    Ok(())
}

impl WasmSimulator {
    /// The machine, or a JS error if this simulator has none.
    ///
    /// Prefer this over `self.machine.as_ref().unwrap()` in anything reachable
    /// from JS. A panic unwinds straight out of the wasm frame as a JS
    /// exception, and JS exceptions do NOT run Rust destructors — the
    /// wasm-bindgen borrow guard never drops and every later call fails with
    /// "recursive use of an object". An `Err` return is an ordinary Rust
    /// return: the guard drops, the glue throws afterwards, and the caller sees
    /// one honest failure instead of a permanently bricked simulator.
    fn machine_or_err(&self) -> Result<&Machine<Box<dyn Cpu>>, JsValue> {
        self.machine
            .as_ref()
            .ok_or_else(|| JsValue::from_str("simulator has no machine"))
    }
}

#[wasm_bindgen]
impl WasmSimulator {
    /// Legacy constructor: hardcoded STM32F107 Cortex-M3 with 128KB flash + 20KB RAM.
    /// Kept for backward compatibility with the existing landing page sandbox.
    #[wasm_bindgen(constructor)]
    pub fn new(firmware: &[u8]) -> Result<WasmSimulator, JsValue> {
        // The fidelity log is THREAD-LOCAL, and the browser builds one machine
        // after another on the same thread — open a second lab and it would
        // inherit the first one's undecoded instructions. Scope it here so
        // `fidelity_gaps()` always describes the machine you are looking at.
        labwired_core::fidelity::reset();
        let mut bus = SystemBus::new();
        bus.flash = LinearMemory::new(128 * 1024, 0x0800_0000);
        bus.ram = LinearMemory::new(20 * 1024, 0x2000_0000);
        bus.refresh_peripheral_index();

        let console = ConsoleCapture::new(HostConsole::Undeclared, HostConsole::UsbSerialJtag);
        let uart_sink = console.heard_sink();
        bus.attach_uart_tx_sink(uart_sink.clone(), false);
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let mut machine = Machine::new(boxed, bus);

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        machine
            .load_firmware(&program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io: Vec::new(),
            uart_sink,
            console,
            uart_rx_bufs,
            arch: MachineFamily::CortexM,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// Config-driven constructor: initialize from system YAML, chip YAML, and firmware ELF.
    ///
    /// Dispatches on `chip.arch`:
    ///   * `Arm` → `SystemBus::from_config` + `configure_cortex_m` (existing path).
    ///   * `Xtensa` → `configure_xtensa_esp32` + inline external-device attach.
    ///     ESP32 chip YAMLs declare RAM banks (IRAM/DRAM/flash XIP/ROM) via
    ///     `peripherals: [{type: ram, ...}]`, which `from_config` doesn't
    ///     understand — it'd stub them out and break instruction fetch. So
    ///     ESP32 takes the dedicated path that explicitly registers those
    ///     banks before attaching SPI / I²C external devices.
    #[wasm_bindgen]
    pub fn new_from_config(
        system_yaml: &str,
        chip_yaml: &str,
        firmware: &[u8],
        blobs: JsValue,
    ) -> Result<WasmSimulator, JsValue> {
        // Same reason as `new()`: the fidelity log is thread-local and outlives
        // the machine, so scope it to this one.
        labwired_core::fidelity::reset();
        let manifest: SystemManifest = serde_yaml::from_str(system_yaml)
            .map_err(|e| JsValue::from_str(&format!("System YAML error: {}", e)))?;
        let chip: ChipDescriptor = serde_yaml::from_str(chip_yaml)
            .map_err(|e| JsValue::from_str(&format!("Chip YAML error: {}", e)))?;

        // The dispatch is `arch_policy`'s, not this crate's. It used to be a
        // local match that folded `Unknown` in with `Arm`, so a chip declaring
        // no architecture ran here as a Cortex-M while the CLI refused it.
        let family = machine_family(&chip)
            .map_err(|e| JsValue::from_str(&format!("Chip architecture error: {e:#}")))?;
        match family {
            MachineFamily::CortexM => Self::new_from_config_arm(&chip, &manifest, firmware),
            MachineFamily::RiscV => {
                let blob_map = parse_named_blobs(&blobs);
                // A board opts into faithful ROM boot by supplying the merged
                // flash image (`bootloader@0x0 + partition-table@0x8000 +
                // app@0x10000`) as the `esp32c3_flash` blob — the same on-demand
                // named-blob idiom the ROM images already use. Its presence is
                // the trigger (no schema flag needed): with it, the browser boots
                // the real mask ROM from the reset vector exactly like the native
                // `--rom-boot` CLI; without it, the pre-existing fast-boot path
                // runs, treating `firmware` as a bare esp-hal ELF.
                if blob_map.contains_key("esp32c3_flash")
                    && blob_map.contains_key(ESP32C3_FLASH_FAST_START_BLOB)
                {
                    Self::new_from_config_riscv_flash_fastboot(&chip, &manifest, &blob_map)
                } else if blob_map.contains_key("esp32c3_flash") {
                    Self::new_from_config_riscv_romboot(&chip, &manifest, &blob_map)
                } else {
                    Self::new_from_config_riscv(&chip, &manifest, firmware, &blob_map)
                }
            }
            MachineFamily::Xtensa if chip.is_esp32s3() => {
                let blob_map = parse_named_blobs(&blobs);
                // Same trigger as the C3: the merged flash image
                // (`bootloader@0x0 + partition-table@0x8000 + app@0x10000`)
                // arriving as a named blob means boot the real mask ROM from
                // the reset vector. Without it, `firmware` is a bare esp-hal
                // ELF and the pre-existing fast-boot path runs.
                //
                // The hosted compiler ships flash images and NO ELF, so before
                // this branch existed every hosted S3 run fell into fast_boot
                // with nothing to load: the mask ROM printed its banner, jumped
                // to the 2nd-stage bootloader, and the app never ran.
                if blob_map.contains_key("esp32s3_flash") {
                    Self::new_from_config_xtensa_esp32s3_flash(&chip, &manifest, &blob_map)
                } else {
                    Self::new_from_config_xtensa_esp32s3(&manifest, firmware, &blob_map)
                }
            }
            MachineFamily::Xtensa => Self::new_from_config_xtensa_esp32(&manifest, firmware),
            // Classic Arduino Nano / ATmega328P — same shape as `build_avr_node`.
            MachineFamily::Avr => Self::new_from_config_avr(&chip, &manifest, firmware),
        }
    }

    fn new_from_config_arm(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        firmware: &[u8],
    ) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        let console = ConsoleCapture::for_manifest(manifest);
        let uart_sink = console.heard_sink();
        bus.attach_host_console(console.tapped(), uart_sink.clone())
            .map_err(|e| JsValue::from_str(&e))?;
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let mut machine = Machine::new(boxed, bus);

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        machine
            .load_firmware(&program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            console,
            uart_rx_bufs,
            arch: MachineFamily::CortexM,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// AVR8 (ATmega328P / classic Arduino Nano) constructor for the browser.
    /// Mirrors `labwired_core::system::node::build_avr_node`: parse ELF into
    /// the Avr program image, attach the host console, box the CPU.
    fn new_from_config_avr(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        firmware: &[u8],
    ) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        let console = ConsoleCapture::for_manifest(manifest);
        let uart_sink = console.heard_sink();
        bus.attach_host_console(console.tapped(), uart_sink.clone())
            .map_err(|e| JsValue::from_str(&e))?;
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        let mut cpu = labwired_core::cpu::Avr::new();
        cpu.load_program_image(&program_image);
        // SPI/I2C kits park on bus controllers; SPDR/TWCR clock them from
        // the CPU model (same as build_avr_node / CLI).
        for name in ["spi", "spi0", "spi1"] {
            for dev in bus.take_spi_devices(name) {
                cpu.push_spi_device(dev);
            }
        }
        for name in ["i2c", "i2c0", "twi"] {
            for dev in bus.take_i2c_slaves(name) {
                cpu.push_i2c_slave(dev);
            }
        }
        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let machine = Machine::new(boxed, bus);

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            console,
            uart_rx_bufs,
            arch: MachineFamily::Avr,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// ONE home for the console decision on the two ESP32-C3 merged-flash paths
    /// (rom-boot and flash fast-start) — the paths a hosted Arduino/ESP-IDF
    /// build actually takes.
    ///
    /// These are the only paths where a real mask ROM runs, and the C3 BROM
    /// prints its banner to UART0 AND USB-Serial-JTAG. Wiring both into one
    /// buffer would render every ROM character twice, so exactly one console is
    /// shown — which is also what a real board gives you, since the socket is
    /// soldered to one of them. The other console is recorded but not shown, so
    /// [`WasmSimulator::console_mismatch`] can explain a pane that stays empty
    /// because the firmware printed to the console this board has no cable on.
    ///
    /// Returns the capture (its `heard_sink` is the Serial pane) plus the sink
    /// to hand `RomBootOpts::usb_serial_sink` — the USB-Serial-JTAG model is
    /// added by the shared core builder AFTER the bus is handed over, so it is
    /// the one console that cannot be attached through the bus here.
    fn attach_c3_flash_console(
        bus: &mut SystemBus,
        manifest: &SystemManifest,
    ) -> Result<C3FlashConsole, JsValue> {
        let console = ConsoleCapture::for_manifest(manifest);
        if console.tapped().is_usb_serial_jtag() {
            // `deploy.usb: native` board (ESP32-C3 SuperMini): the USB-C socket
            // IS the C3's USB-Serial-JTAG. UART0 exists and the ROM still writes
            // to it, but on this board it comes out on GPIO20/21 header pins with
            // nothing attached — record it as the console nobody can hear.
            bus.attach_uart_tx_sink(console.unheard_sink(), false);
            let usb = console.heard_sink();
            Ok((console, usb))
        } else {
            // Bridge-chip board, or an undeclared manifest (historical default:
            // UART0, where every Arduino/IDF lab shipped so far prints).
            bus.attach_host_console(console.tapped(), console.heard_sink())
                .map_err(|e| JsValue::from_str(&e))?;
            let usb = console.unheard_sink();
            Ok((console, usb))
        }
    }

    /// RISC-V (esp32c3) bus setup. Mirrors `new_from_config_arm` but builds a
    /// RISC-V core via `configure_riscv` and seeds the stack pointer at the top
    /// of DRAM — fast-boot skips the ROM/2nd-stage bootloader that would
    /// normally set SP, so the app's first prologue store would otherwise fault.
    ///
    /// The ESP32-C3 boot ROM is injected on demand via `blobs` under
    /// `esp32c3_irom`/`esp32c3_drom` — the RISC-V analogue of the S3 path's
    /// `Esp32s3Opts.rom_images`. The chip YAML declares zero-filled `rom` /
    /// `rom_data` regions (IROM 0x4000_0000, DROM 0x3FF0_0000) that native
    /// builds fill from env pins or the vendored images; on wasm the vendored
    /// images are excluded from the bundle, so the browser fetches the two ROM
    /// bins and passes them here. With the ROM present, esp-hal's ROM function
    /// calls during init resolve for real (zero thunks) instead of dispatching
    /// through zeros.
    fn new_from_config_riscv(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        firmware: &[u8],
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        Self::new_from_config_riscv_program_image(chip, manifest, &program_image, blobs)
    }

    /// Attach every real WiFi MAC to a per-lab virtual-WiFi medium built from the
    /// manifest's `wifi_ap`. Delegates to the shared, CPU-generic core helper so
    /// the browser ctors and the CLI `test`/`run` paths attach identically — the
    /// universal-WiFi-adapter plumbing lives in exactly one place.
    fn attach_wifi_ap(machine: &mut Machine<Box<dyn Cpu>>, manifest: &SystemManifest) {
        labwired_core::system::wifi::attach_configured_wifi_ap(&mut machine.bus, manifest);
    }

    fn new_from_config_riscv_flash_fastboot(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        use labwired_core::boot::esp32c3_rom::{
            build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
        };
        use labwired_core::boot::esp32s3_rom::RomImages;

        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        let (Some(irom), Some(drom)) = (blobs.get("esp32c3_irom"), blobs.get("esp32c3_drom"))
        else {
            return Err(JsValue::from_str(
                "C3 flash fast-start needs ESP32-C3 ROM blobs: pass esp32c3_irom + esp32c3_drom",
            ));
        };
        let images = RomImages {
            irom: irom.clone(),
            drom: drom.clone(),
        };
        if !inject_rom_regions(&mut bus, &images) {
            return Err(JsValue::from_str(
                "C3 flash fast-start: chip YAML declares no IROM region at 0x40000000",
            ));
        }
        // The bootloader calls ROM helpers through DRAM tables initialized by
        // the mask ROM reset code. Because this path skips that reset code, copy
        // those ROM `.data` records before entering the second-stage bootloader.
        for (dst, bytes) in c3_rom_data_init_writes(irom) {
            for (i, b) in bytes.iter().enumerate() {
                let _ = bus.write_u8(dst as u64 + i as u64, *b);
            }
        }

        let flash = blobs
            .get("esp32c3_flash")
            .ok_or_else(|| JsValue::from_str("fast-start needs esp32c3_flash"))?;
        let bootloader_image = esp32c3_bootloader_program_image_from_merged_flash(flash)
            .map_err(|e| JsValue::from_str(&format!("ESP32-C3 flash fast-start: {e}")))?;

        let (console, usb_serial_sink) = Self::attach_c3_flash_console(&mut bus, manifest)?;
        let uart_sink = console.heard_sink();
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let mut machine = build_rom_boot_machine(
            bus,
            flash.clone(),
            RomBootOpts {
                // A new die per bridge. Two MCUs on one canvas are two dies, and
                // the browser builds one bridge each, so leaving this unpinned is
                // what gives them distinct WiFi station MACs and BLE addresses.
                pinned_efuse_mac: None,
                usb_serial_sink: Some(usb_serial_sink),
            },
            |c| Box::new(c) as Box<dyn Cpu>,
        );
        load_program_segments_without_reset(&mut machine, &bootloader_image)
            .map_err(|e| JsValue::from_str(&format!("C3 flash fast-start load: {e}")))?;

        let sp_top = (chip.ram.base + chip.ram.size) as u32;
        machine.cpu.set_sp(sp_top & !0xF);
        machine.cpu.set_pc(bootloader_image.entry_point as u32);

        // Attach the per-lab virtual-WiFi AP if the diagram declares one. This is
        // the fix for the fast-start path silently lacking WiFi (the browser's
        // default C3 path) while rom-boot had it.
        Self::attach_wifi_ap(&mut machine, manifest);

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            console,
            uart_rx_bufs,
            arch: MachineFamily::RiscV,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    fn new_from_config_riscv_program_image(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        program_image: &ProgramImage,
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        // Inject the on-demand ESP32-C3 boot ROM blobs into the chip's still
        // zero-filled `rom`/`rom_data` regions, matching how the native
        // `--rom-boot` path (`build_c3_rom_boot_machine`) provisions them.
        // Absent blobs (non-C3 RISC-V chips, or the browser not supplying them)
        // leave the regions zero, preserving the pre-existing fast-boot path.
        let faithful_c3_rom = {
            use labwired_core::boot::esp32c3_rom::{c3_rom_data_init_writes, DROM_BASE, IROM_BASE};
            let mut injected_irom: Option<Vec<u8>> = None;
            for mem in bus.extra_mem.iter_mut() {
                let src = if mem.base_addr == IROM_BASE as u64 {
                    blobs.get("esp32c3_irom")
                } else if mem.base_addr == DROM_BASE as u64 {
                    blobs.get("esp32c3_drom")
                } else {
                    None
                };
                if let Some(src) = src {
                    let n = src.len().min(mem.data.len());
                    mem.data[..n].copy_from_slice(&src[..n]);
                    if mem.base_addr == IROM_BASE as u64 {
                        injected_irom = Some(src.clone());
                    }
                }
            }
            // Fast-boot skips the ROM reset's own `.data` copy, so replicate it:
            // land the ROM's DRAM globals (ROM function tables esp-hal calls
            // dispatch through) exactly as silicon does — otherwise those calls
            // jump through a null/garbage pointer. Mirrors the S3 path's
            // `s3_rom_data_init_writes` in `configure_xtensa_esp32s3`.
            if let Some(irom) = injected_irom {
                for (dst, bytes) in c3_rom_data_init_writes(&irom) {
                    for (i, b) in bytes.iter().enumerate() {
                        let _ = bus.write_u8(dst as u64 + i as u64, *b);
                    }
                }
                // With the real ROM present, esp-hal's clock bring-up runs the
                // genuine `rom_i2c_*Reg` helpers, which drive the analog I²C
                // master / ANA_CONFIG block (0x6000_E000) for the PLL. That
                // block is not in the chip YAML (it's the same custom model the
                // native `--rom-boot` builder wires), so add it here on the
                // faithful path — otherwise the first ROM PLL transaction faults
                // on an unmapped access. Its FSM-status model lets the ROM's
                // transaction busy-poll complete.
                bus.add_peripheral(
                    "rtc_i2c_ana",
                    0x6000_E000,
                    0x400,
                    None,
                    Box::new(labwired_core::peripherals::esp32c3::ana_i2c::Esp32c3AnaI2c::new()),
                );
                bus.refresh_peripheral_index();
                // A peripheral added AFTER bus assembly changes the input to
                // `derive_walk_deletable`, which the boot path already ran. It
                // is correct today only because this model happens to be inert
                // — the walk-deletion flag would silently disagree with the
                // live peripheral set the moment it stopped being. Re-derive
                // rather than rely on that. (Cheap: one pass over the roster,
                // once per simulator construction.)
                bus.recompute_walk_deletable();
                true
            } else {
                false
            }
        };

        let console = ConsoleCapture::for_manifest(manifest);
        let uart_sink = console.heard_sink();
        // On the faithful C3 ROM path, esp-println's `jtag-serial` feature (used
        // by esp-hal apps) prints through USB_SERIAL_JTAG (0x6004_3000), not
        // UART0. The chip YAML only has a declarative register stub there, which
        // never drains bytes, so install the real behavioral model (same IP as
        // the S3, reused unchanged). A narrower, later-registered window
        // overrides the declarative stub.
        if faithful_c3_rom {
            use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
            // `new_esp32c3()`, not `new()`: the latter leaves irq_source None, so
            // the CDC interrupt never reaches the matrix and a CDC-on-boot build
            // prints nothing. The sink is NOT attached here any more — the
            // console-selection path below routes it via
            // `attach_usb_serial_jtag_sink` so the tap follows the board's real
            // USB socket instead of being hard-wired at construction.
            bus.add_peripheral(
                "usb_serial_jtag",
                0x6004_3000,
                0x100,
                None,
                Box::new(UsbSerialJtag::new_esp32c3()),
            );
            bus.refresh_peripheral_index();
            // Same reason as the `rtc_i2c_ana` addition above: re-derive
            // walk-deletion over the peripheral set that actually exists, not
            // the one the boot path saw.
            bus.recompute_walk_deletable();
        }
        // No mask ROM executes on this bare-ELF path, so nothing writes the same
        // bytes to both consoles: an undeclared manifest can keep capturing both
        // into one pane, exactly as before. A manifest that DOES declare the
        // board's console is authoritative and selects it — same rule, same
        // parser, as the merged-flash paths.
        match console.tapped() {
            HostConsole::Undeclared => {
                if faithful_c3_rom {
                    bus.attach_usb_serial_jtag_sink(uart_sink.clone());
                }
                bus.attach_uart_tx_sink(uart_sink.clone(), false);
            }
            tapped => {
                if faithful_c3_rom && !tapped.is_usb_serial_jtag() {
                    bus.attach_usb_serial_jtag_sink(console.unheard_sink());
                }
                bus.attach_host_console(tapped, uart_sink.clone())
                    .map_err(|e| JsValue::from_str(&e))?;
                if tapped.is_usb_serial_jtag() {
                    bus.attach_uart_tx_sink(console.unheard_sink(), false);
                }
            }
        }
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let mut machine = Machine::new(boxed, bus);

        machine
            .load_firmware(program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;

        let sp_top = (chip.ram.base + chip.ram.size) as u32;
        machine.cpu.set_sp(sp_top & !0xF);
        machine.cpu.set_pc(program_image.entry_point as u32);

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            console,
            uart_rx_bufs,
            arch: MachineFamily::RiscV,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// RISC-V (ESP32-C3) FAITHFUL ROM-boot path — the browser analogue of the
    /// native CLI `--rom-boot`. Unlike fast-boot (which jumps straight to an ELF
    /// app entry), this resets to the BROM vector `0x4000_0000` and runs the
    /// genuine mask ROM → 2nd-stage bootloader → `app_main()`, loading from a
    /// merged flash image. Arduino/ESP-IDF images are flash images that run from
    /// flash via cache/XIP, so they REQUIRE this sequence.
    ///
    /// Blobs (all fetched on demand, none baked into the wasm bundle):
    ///   * `esp32c3_irom` / `esp32c3_drom` — the boot ROM images, injected into
    ///     the chip's zero-filled `rom`/`rom_data` regions.
    ///   * `esp32c3_flash` — the merged flash image; this is the actual program.
    ///
    /// All peripheral wiring + reset-vector boot is the shared core builder
    /// [`labwired_core::boot::esp32c3_rom::build_rom_boot_machine`], byte-for-byte
    /// the same machine the native CLI assembles. Zero thunks.
    fn new_from_config_riscv_romboot(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        use labwired_core::boot::esp32c3_rom::{
            build_rom_boot_machine, inject_rom_regions, RomBootOpts,
        };
        use labwired_core::boot::esp32s3_rom::RomImages;

        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        // Provision the boot ROM into the chip's zero-filled rom/rom_data
        // regions (the native path fills them from env pins / vendored images;
        // on wasm the browser fetches and passes the two bins). ROM-boot cannot
        // proceed without the real ROM — the reset vector executes it directly.
        let (Some(irom), Some(drom)) = (blobs.get("esp32c3_irom"), blobs.get("esp32c3_drom"))
        else {
            return Err(JsValue::from_str(
                "rom-boot needs the ESP32-C3 boot ROM: pass esp32c3_irom + esp32c3_drom blobs",
            ));
        };
        let images = RomImages {
            irom: irom.clone(),
            drom: drom.clone(),
        };
        if !inject_rom_regions(&mut bus, &images) {
            return Err(JsValue::from_str(
                "rom-boot: chip YAML declares no IROM region at 0x40000000 to load the boot ROM",
            ));
        }

        let flash_bytes = blobs
            .get("esp32c3_flash")
            .expect("esp32c3_flash presence checked by caller")
            .clone();

        let (console, usb_serial_sink) = Self::attach_c3_flash_console(&mut bus, manifest)?;
        let uart_sink = console.heard_sink();
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let mut machine = build_rom_boot_machine(
            bus,
            flash_bytes,
            RomBootOpts {
                // A new die per bridge — see the fast-start path above.
                pinned_efuse_mac: None,
                usb_serial_sink: Some(usb_serial_sink),
            },
            // WasmSimulator holds Machine<Box<dyn Cpu>>; box the concrete RiscV.
            |c| Box::new(c) as Box<dyn Cpu>,
        );

        // Attach the per-lab virtual-WiFi AP if the diagram declares one (shared
        // with the flash-fast-start path — ONE source of truth).
        Self::attach_wifi_ap(&mut machine, manifest);

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            console,
            uart_rx_bufs,
            arch: MachineFamily::RiscV,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// ESP32-classic (Xtensa LX6) bus setup. `configure_xtensa_esp32` adds
    /// IRAM / DRAM / flash XIP / ROM / UART0; external device attach
    /// (SSD1680 e-paper etc) is handled by the core helper since this code
    /// path doesn't go through `SystemBus::from_config`.
    fn new_from_config_xtensa_esp32(
        manifest: &SystemManifest,
        firmware: &[u8],
    ) -> Result<WasmSimulator, JsValue> {
        // Drop any leftover process/thread-local aids state from a prior
        // WasmSimulator in this worker (re-run / lab switch). See
        // `rom_thunks::reset_esp32_session_state`.
        labwired_core::peripherals::esp_xtensa_common::rom_thunks::reset_esp32_session_state();

        let mut bus = SystemBus::new();
        let cpu = configure_xtensa_esp32(&mut bus);

        // A classic ESP32 has NO USB peripheral: its devkit's CP210x sits on
        // UART0 and IS the USB device the host enumerates. So `debug_uart:
        // usb_serial_jtag` here is a board-mapping error, and `attach_host_console`
        // says so instead of quietly showing UART0 under a USB label.
        let console = ConsoleCapture::for_manifest(manifest);
        let uart_sink = console.heard_sink();
        bus.attach_host_console(console.tapped(), uart_sink.clone())
            .map_err(|e| JsValue::from_str(&e))?;
        let uart_rx_bufs = bus.attach_uart_rx_source();

        labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, manifest)
            .map_err(|e| JsValue::from_str(&format!("ESP32 external_devices: {:#}", e)))?;
        bus.refresh_peripheral_index();

        let boxed: Box<dyn Cpu> = Box::new(cpu);
        // Real dual-core: attach a second LX6 as APP_CPU (PRID 0xABAB → core 1,
        // starts halted until PRO_CPU releases it via ets_set_appcpu_boot_addr).
        // This replaces the old single-core handshake-forging stub: loopTask
        // (pinned to CONFIG_ARDUINO_RUNNING_CORE=1) now runs on a genuine
        // second core, and the cross-core yield IPI is delivered by the core's
        // DPORT through Machine::step — see crates/core/tests/e2e_labwired_ereader.rs.
        let app_cpu: Box<dyn Cpu> = Box::new(labwired_core::cpu::XtensaLx7::new_app_cpu());
        let mut machine = Machine::new(boxed, bus).with_secondary_cpu(app_cpu);

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        machine
            .load_firmware(&program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;
        // XtensaLx7::reset() defaults PC to 0x40000400 (BROM reset vector).
        // We skip BROM emulation and jump straight to the ELF's app entry,
        // matching where a 2nd-stage bootloader would land.
        machine.cpu.set_pc(program_image.entry_point as u32);
        // BROM seeds SP near top of DRAM before call_start_cpu0; we skip BROM,
        // so seed both cores' stacks (APP_CPU in a separate DRAM region below
        // PRO_CPU's), matching the native dual-core bring-up.
        machine.cpu.set_sp(0x3FFE_0000);
        if let Some(cpu1) = machine.cpu_secondary.as_mut() {
            cpu1.set_sp(0x3FFD_8000);
        }

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            console,
            uart_rx_bufs,
            arch: MachineFamily::Xtensa,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// ESP32-S3 (Xtensa LX7) bus setup — the FAITHFUL fast-boot path.
    ///
    /// `configure_xtensa_esp32s3` installs IRAM/DRAM/RTC/flash-XIP plus the
    /// real boot ROM (zero thunks; the ROM `.data` init lands
    /// `rom_cache_internal_table_ptr` so esp-hal's ROM cache calls run for
    /// real). The ROM is NOT baked into the wasm bundle — it is fetched on
    /// demand and passed in `blobs` under `esp32s3_irom`/`esp32s3_drom`, then
    /// injected via `Esp32s3Opts.rom_images`. `fast_boot` then loads the app
    /// ELF's segments (identity XIP) and synthesises post-bootloader CPU state.
    /// Serial output on the S3 esp-hal apps goes through USB_SERIAL_JTAG, so we
    /// route that peripheral's sink into the `uart_sink` the widget reads.
    /// Faithful ESP32-S3 boot from a merged flash image, with **no ELF**.
    ///
    /// The Xtensa counterpart of `new_from_config_riscv_flash_fastboot`, and
    /// the path every hosted S3 run needs: the hosted compiler produces flash
    /// images (bootloader + partition table + app) and no ELF, so a
    /// constructor that can only `fast_boot(elf)` has nothing to boot. What
    /// that produced looked exactly like a hang — the mask ROM printed its
    /// banner, jumped to the 2nd-stage bootloader, and stopped.
    ///
    /// The assembly is the native `--rom-boot` sequence, already proven on this
    /// chip: `real_reset_boot` selects the MMU XIP model (both cache windows
    /// alias one physical flash backing and translate through the table the
    /// bootloader programs — identity XIP reads the wrong page and returns
    /// zeros), the flash image is passed as bytes rather than through
    /// `LABWIRED_ESP32S3_FLASH` (there is no env on wasm), and the CPU is left
    /// at the BROM reset vector so the chip's own ROM loads the app and jumps
    /// to it. No `fast_boot`, no synthesised post-bootloader state, no thunks.
    fn new_from_config_xtensa_esp32s3_flash(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        use labwired_core::boot::esp32s3_rom::RomImages;
        use labwired_core::system::xtensa::{
            configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts,
        };

        let flash = blobs.get("esp32s3_flash").ok_or_else(|| {
            JsValue::from_str("ESP32-S3 flash boot needs the merged flash image blob esp32s3_flash")
        })?;

        // The real ROM is not optional here. Fast-boot may fall back to the
        // thunk harness because it jumps straight into the app; this path IS
        // the ROM, so a missing blob has to say so rather than boot nothing.
        let (Some(irom), Some(drom)) = (blobs.get("esp32s3_irom"), blobs.get("esp32s3_drom"))
        else {
            return Err(JsValue::from_str(
                "ESP32-S3 flash boot needs the boot ROM blobs: pass esp32s3_irom + esp32s3_drom",
            ));
        };

        let mut bus = SystemBus::new();
        let opts = Esp32s3Opts {
            real_reset_boot: true,
            rom_images: Some(RomImages {
                irom: irom.clone(),
                drom: drom.clone(),
            }),
            flash_image: Some(flash.clone()),
            // Size the backing from the CHIP descriptor, exactly like the
            // native `--rom-boot` CLI (`commands/run.rs`) — never from the
            // image's own byte count. The part's capacity is a property of the
            // module, not of how much of it this build happens to fill, and the
            // model publishes it as the JEDEC capacity byte
            // (`spi_mem_flash.rs` CMD_RDID: `log2(backing.len())`). Sizing to
            // the image made an 8,455,860-byte N16R8 image report an 8 MiB part
            // while its own header declares 16 MB, and esp_flash refuses to
            // boot on the mismatch:
            //   E spi_flash: Detected size(8192k) smaller than the size in the
            //   binary image header(16384k). Probe failed.
            // The `.max(image len)` floor stays so a chip YAML that understates
            // the part still cannot truncate the image itself.
            flash_size: esp32s3_flash_backing_size(chip.flash.size, flash.len()),
            ..Esp32s3Opts::default()
        };
        let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
        if wiring.boot_mode != Esp32s3BootMode::Faithful {
            return Err(JsValue::from_str(
                "ESP32-S3 flash boot needs the real boot ROM, but the injected images did not resolve",
            ));
        }
        let mut cpu = wiring.cpu;
        // Same reason the native `--rom-boot` CLI and `build_esp32s3_node` set
        // it: the ROM and the app install the window overflow/underflow vectors
        // and build a genuine stack save chain, so the CPU must use the real
        // per-access spill/fill path rather than the simulator's shadow stack.
        // This constructor was the only rom-boot entry point that left it off —
        // an ESP-IDF app with deep call chains then faulted on a garbage
        // address restored from the shadow stack.
        cpu.faithful_windows = true;
        // Read it back rather than re-typing `true` below: the APP core must
        // use the SAME window-handling mode as the PRO core, and the only way
        // that stays true through a later edit is to derive it from one place.
        let primary_faithful_windows = cpu.faithful_windows;

        // Console selection is the rule every other ESP path uses: an
        // undeclared manifest hears both consoles in one pane, a declared one
        // is authoritative. Both taps matter here — the mask ROM prints on
        // UART0 while an Arduino sketch built CDC-on-boot prints on
        // USB-Serial-JTAG, so the boot banner and the sketch arrive on
        // different peripherals of the same run.
        let console = ConsoleCapture::for_manifest(manifest);
        let uart_sink = console.heard_sink();
        match console.tapped() {
            HostConsole::Undeclared => {
                bus.attach_usb_serial_jtag_sink(uart_sink.clone());
                bus.attach_uart_tx_sink(uart_sink.clone(), false);
            }
            tapped => {
                if !tapped.is_usb_serial_jtag() {
                    bus.attach_usb_serial_jtag_sink(console.unheard_sink());
                }
                bus.attach_host_console(tapped, uart_sink.clone())
                    .map_err(|e| JsValue::from_str(&e))?;
                if tapped.is_usb_serial_jtag() {
                    bus.attach_uart_tx_sink(console.unheard_sink(), false);
                }
            }
        }
        let uart_rx_bufs = bus.attach_uart_rx_source();

        labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, manifest)
            .map_err(|e| JsValue::from_str(&format!("ESP32-S3 external_devices: {:#}", e)))?;
        bus.refresh_peripheral_index();

        let boxed: Box<dyn Cpu> = Box::new(cpu);
        // Real second core. An ESP-IDF image built dual-core (the default)
        // stops dead at `cpu_start: Multicore app` without one: PRO_CPU spins in
        // `main_task` on `s_other_cpu_startup_done`, which only the APP_CPU idle
        // hook can set. The core starts halted and is released by the hardware
        // edge the firmware drives (`SYSTEM_CORE_1_CONTROL_0.RESETING` 1->0),
        // exactly as the native runner and `system::node::build_esp32s3_node`
        // do — no forged handshake flags.
        let mut app_cpu_lx7 = labwired_core::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();
        // ⚠️ The APP core needs `faithful_windows` for exactly the same reason
        // the PRO core does, and this constructor set it on only one of them.
        // Core 1 boots the real ROM from its own reset vector and then runs the
        // same ESP-IDF image, so it spills and fills register windows through
        // the firmware's own OF/UF vectors; left on the simulator shadow stack
        // it restored a garbage SP and every window overflow stored near
        // address 0 (`Memory access violation at 0xffffffe0`).
        //
        // That fault was INVISIBLE from the browser: `Sim::step_batch` reports
        // `Ok(elapsed)` whenever the primary retired at least one cycle, so a
        // secondary faulting on EVERY machine boundary looked like steady
        // forward progress while core 1 never executed a single instruction of
        // firmware. PRO_CPU then spins forever in
        // `spi_flash_disable_interrupts_caches_and_other_cpu` waiting for a
        // `spi_flash_op_block_func` on core 1 that can never run. The native
        // `--rom-boot` runner has always set both (`commands/run.rs`), which is
        // why this was a browser-only hang.
        app_cpu_lx7.faithful_windows = primary_faithful_windows;
        let app_cpu: Box<dyn Cpu> = Box::new(app_cpu_lx7);
        let machine = Machine::new(boxed, bus).with_secondary_cpu(app_cpu);

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io: manifest.board_io.clone(),
            uart_sink,
            console,
            uart_rx_bufs,
            arch: MachineFamily::Xtensa,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    fn new_from_config_xtensa_esp32s3(
        manifest: &SystemManifest,
        firmware: &[u8],
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
        use labwired_core::boot::esp32s3_rom::RomImages;
        use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};

        // Inject the on-demand ROM blobs (None → configure falls back to the
        // native provision chain, which is None on wasm → thunk harness).
        let rom_images = match (blobs.get("esp32s3_irom"), blobs.get("esp32s3_drom")) {
            (Some(irom), Some(drom)) => Some(RomImages {
                irom: irom.clone(),
                drom: drom.clone(),
            }),
            _ => None,
        };

        let mut bus = SystemBus::new();
        // Default XIP model (fast-boot identity; --rom-boot's MMU model is
        // native-CLI only) + the injected faithful ROM.
        let opts = Esp32s3Opts {
            rom_images,
            ..Esp32s3Opts::default()
        };
        let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
        let mut cpu = wiring.cpu;

        // S3 fast-boot runs no mask ROM, so nothing writes the same bytes to
        // both consoles: an undeclared manifest keeps capturing both into one
        // pane, exactly as before (esp-hal's `esp_println` targets
        // USB_SERIAL_JTAG; an Arduino sketch may use UART0). A manifest that
        // declares the board's console is authoritative and selects it — the
        // same rule and the same parser as the C3 merged-flash paths.
        let console = ConsoleCapture::for_manifest(manifest);
        let uart_sink = console.heard_sink();
        match console.tapped() {
            HostConsole::Undeclared => {
                bus.attach_usb_serial_jtag_sink(uart_sink.clone());
                bus.attach_uart_tx_sink(uart_sink.clone(), false);
            }
            tapped => {
                if !tapped.is_usb_serial_jtag() {
                    bus.attach_usb_serial_jtag_sink(console.unheard_sink());
                }
                bus.attach_host_console(tapped, uart_sink.clone())
                    .map_err(|e| JsValue::from_str(&e))?;
                if tapped.is_usb_serial_jtag() {
                    bus.attach_uart_tx_sink(console.unheard_sink(), false);
                }
            }
        }
        let uart_rx_bufs = bus.attach_uart_rx_source();

        // Wire any devices the manifest declares (e.g. an SH1107 OLED on i2c0) —
        // the same factory the classic-ESP32 and native builder paths use. Without
        // this, an S3 board's `external_devices` were silently dropped and the
        // panel never rendered. Connect the blocks the manifest says are wired.
        labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, manifest)
            .map_err(|e| JsValue::from_str(&format!("ESP32-S3 external_devices: {:#}", e)))?;
        bus.refresh_peripheral_index();

        fast_boot(
            firmware,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
                icache_backing: Some(wiring.icache_backing),
                dcache_backing: Some(wiring.dcache_backing),
                factory_flash_base: None,
            },
        )
        .map_err(|e| JsValue::from_str(&format!("ESP32-S3 fast_boot: {e}")))?;

        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let machine = Machine::new(boxed, bus);

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io: manifest.board_io.clone(),
            uart_sink,
            console,
            uart_rx_bufs,
            arch: MachineFamily::Xtensa,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    fn machine(&mut self) -> &mut Machine<Box<dyn Cpu>> {
        self.machine.as_mut().unwrap()
    }

    /// Read the output state of a board_io binding using peripheral snapshot.
    fn read_board_io_state(
        &self,
        machine: &Machine<Box<dyn Cpu>>,
        binding: &BoardIoBinding,
    ) -> bool {
        let idx = match machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
        {
            Some(i) => i,
            None => return false,
        };

        let pin_high = match binding.kind {
            BoardIoKind::Led | BoardIoKind::PwmOutput => machine.bus.peripherals[idx]
                .dev
                .read_gpio_output(binding.pin)
                .unwrap_or(false),
            BoardIoKind::Button => machine.bus.peripherals[idx]
                .dev
                .read_gpio_input(binding.pin)
                .unwrap_or(false),
            // Analog/bus kinds are not boolean and are exposed through typed state accessors.
            BoardIoKind::AdcInput
            | BoardIoKind::I2cDevice
            | BoardIoKind::SpiDevice
            | BoardIoKind::UartDevice => {
                return false;
            }
        };

        if binding.active_high {
            pin_high
        } else {
            !pin_high
        }
    }

    /// Browser-side GDB stub entry point.
    ///
    /// Disabled in this build: the GdbStub `Target` impl in `labwired-gdbstub`
    /// is concrete on `LabwiredTarget<CortexM>` / `LabwiredTarget<RiscV>`,
    /// but `WasmSimulator` now holds `Machine<Box<dyn Cpu>>` so the bound
    /// isn't satisfied. The playground has no JS caller for this method,
    /// so we return an empty packet rather than refactor `labwired-gdbstub`
    /// to be dyn-aware. Track via the v0.6 plan.
    #[wasm_bindgen]
    pub fn gdb_process_packet(&mut self, _packet: &[u8]) -> Vec<u8> {
        Vec::new()
    }

    #[wasm_bindgen]
    pub fn step(&mut self, cycles: u32) -> Result<(), JsValue> {
        for _ in 0..cycles {
            self.machine()
                .advance(AdvanceRequest::single())
                .map_err(|e| JsValue::from_str(&format!("Step Error: {}", e)))?;
        }
        Ok(())
    }

    #[wasm_bindgen]
    pub fn step_single(&mut self) -> Result<(), JsValue> {
        self.machine()
            .advance(AdvanceRequest::single())
            .map(|_| ())
            .map_err(|e| JsValue::from_str(&format!("Step Error: {}", e)))
    }

    /// Connect this chip's UART (`uart_id`, e.g. "uart2") to a shared cross-link
    /// `bus`, so it exchanges bytes with the other chip on the same `link_id`.
    /// The two chips of a point-to-point IO-Link use opposite `side`s (0 and 1)
    /// of the SAME `WireBus`. Bytes flow through the bus with no per-byte host
    /// round-trip, so both chips can keep stepping in batches. Chips wired to
    /// different `WireBus` instances are fully isolated.
    #[wasm_bindgen]
    pub fn attach_uart_wire(
        &mut self,
        uart_id: &str,
        link_id: u32,
        side: u8,
        bus: &WireBus,
    ) -> Result<(), JsValue> {
        let endpoint = Box::new(bus.inner.endpoint(link_id, side));
        self.machine()
            .bus
            .attach_uart_stream_by_id(uart_id, endpoint)
            .map_err(|e| JsValue::from_str(&format!("attach_uart_wire: {e:#}")))?;
        // Keep the cross-link's raw protocol octets out of the human serial
        // monitor — they're decoded by the protocol analyzer (uart_trace), and
        // dumping them into the console floods both peers with identical-looking
        // binary. The debug UART (USART1) still feeds the console normally.
        self.machine()
            .bus
            .detach_uart_sink_by_id(uart_id)
            .map_err(|e| JsValue::from_str(&format!("attach_uart_wire(sink): {e:#}")))
    }

    /// Bind this chip's nRF RADIO + ESP32-C3 BT + cellular modem to a shared
    /// multi-chip [`AirBus`] (browser lab-group). `node_id` is the MCU part id
    /// for path-loss layout and UE identity.
    #[wasm_bindgen]
    pub fn attach_lab_air(&mut self, node_id: &str, air: &AirBus) {
        self.machine().bus.attach_lab_air(
            node_id,
            air.nrf.clone(),
            air.ble.clone(),
            air.cellular.clone(),
        );
    }

    #[wasm_bindgen]
    pub fn get_pc(&self) -> Result<u32, JsValue> {
        Ok(self.machine_or_err()?.cpu.get_pc())
    }

    #[wasm_bindgen]
    pub fn get_register(&self, id: u8) -> Result<u32, JsValue> {
        Ok(self.machine_or_err()?.cpu.get_register(id))
    }

    #[wasm_bindgen]
    pub fn get_register_names(&self) -> Result<JsValue, JsValue> {
        let names = self.machine_or_err()?.cpu.get_register_names();
        serde_wasm_bindgen::to_value(&names)
            .map_err(|error| JsValue::from_str(&format!("register names: {error}")))
    }

    /// Everything this machine failed to model so far, as a flat list of
    /// [`labwired_core::fidelity::FidelityGap`].
    ///
    /// Phases 3.1-3.3 built the census — `record_undecoded` / `record_unmapped`
    /// on the silent paths, `to_gaps()` to flatten it — and then only the CLI
    /// ever read it. `to_gaps` had exactly three callers, all under `crates/cli`,
    /// and the word "fidelity" appeared in this crate only inside comments. So
    /// the engine knew precisely which instructions it had skipped and which
    /// addresses nothing claimed, and the browser — where nearly every user
    /// actually runs a lab — was never told. An undecoded instruction is a
    /// silent no-op that leaves registers stale; it looks exactly like firmware
    /// running correctly.
    ///
    /// Non-draining ON PURPOSE: this reads `report()`, not `take()`. A UI polls,
    /// and `take()` would hand the gaps to whichever poll happened to land first
    /// and show nothing to the next — a warning that blinks out is worse than no
    /// warning. Scoping is done by resetting at construction instead, so the
    /// list always means "gaps for the machine you are looking at".
    #[wasm_bindgen]
    pub fn fidelity_gaps(&self) -> Result<JsValue, JsValue> {
        let gaps = labwired_core::fidelity::report().to_gaps();
        serde_wasm_bindgen::to_value(&gaps)
            .map_err(|error| JsValue::from_str(&format!("fidelity gaps: {error}")))
    }

    // A `fidelity_total_hits() -> u64` companion was written and then removed:
    // it is a bare return type, so `error_boundary_ratchet` counted it as a new
    // failure-blind boundary and went red (77 against a ceiling of 76). That
    // ratchet is correct to complain and the ceiling only shrinks, so raising it
    // for a convenience accessor would be the exact move its doc comment warns
    // against. The count is `gaps.length` / a `reduce` over `count` on the JS
    // side, from data `fidelity_gaps()` already returns.

    /// Read `len` bytes at `addr` through the real bus read path.
    ///
    /// Errors rather than substituting `0` for a byte the bus refused. The old
    /// `unwrap_or(0)` made a failed read byte-identical to a register or memory
    /// cell that genuinely reads zero, and `null`/`0` is exactly the answer a
    /// verdict cannot tell apart from data. `WasmWorld::read_memory` has always
    /// returned `Result`; this brings the single-machine path to the same
    /// contract.
    ///
    /// Note this fires read side effects (it is a bus read, not a peek) — see
    /// [`labwired_core::MachineTrait::read_memory`]. Use `peek`/`inspect` for
    /// anything a human is merely looking at.
    #[wasm_bindgen]
    pub fn read_memory(&self, addr: u32, len: u32) -> Result<Vec<u8>, JsValue> {
        let machine = self.machine_or_err()?;
        (0..len)
            .map(|i| {
                let at = addr as u64 + i as u64;
                machine.bus.read_u8(at).map_err(|error| {
                    JsValue::from_str(&format!("memory read failed at {at:#010x}: {error:?}"))
                })
            })
            .collect()
    }

    #[wasm_bindgen]
    pub fn get_disassembly(&self) -> String {
        let machine = self.machine.as_ref().unwrap();
        let pc = machine.cpu.get_pc();
        match self.arch {
            // ESP32-C3 / generic RV32: use the RISC-V decoder. The previous path
            // always ran Thumb decode, so C3 Trace showed ARM-looking ops and
            // frequent `Unknown32` against real RISC-V encodings.
            MachineFamily::RiscV => {
                let pc = pc & !1;
                match machine.bus.read_u16(pc as u64) {
                    Ok(lo) => {
                        // RV32C: least-significant two bits != 0b11 ⇒ 16-bit.
                        if lo & 0b11 != 0b11 {
                            format!("{:?}", decode_rv32c(lo))
                        } else {
                            match machine.bus.read_u16(pc as u64 + 2) {
                                Ok(hi) => {
                                    let word = (u32::from(hi) << 16) | u32::from(lo);
                                    format!("{:?}", decode_rv32(word))
                                }
                                Err(_) => "?? (Error reading RV hi half)".to_string(),
                            }
                        }
                    }
                    Err(_) => "?? (Error reading RV instruction)".to_string(),
                }
            }
            MachineFamily::Xtensa => {
                // Match the LX7 fetch path: length from byte0, then narrow/wide.
                match machine.bus.read_u8(pc as u64) {
                    Ok(b0) => {
                        let len = xtensa_length::instruction_length(b0);
                        if len == 2 {
                            match machine.bus.read_u16(pc as u64) {
                                Ok(hw) => format!("{:?}", xtensa_narrow::decode_narrow(hw)),
                                Err(_) => "?? (Error reading Xtensa narrow)".to_string(),
                            }
                        } else {
                            match machine.bus.read_u32(pc as u64) {
                                Ok(w) => format!("{:?}", xtensa::decode(w)),
                                Err(_) => "?? (Error reading Xtensa wide)".to_string(),
                            }
                        }
                    }
                    Err(_) => "?? (Error reading Xtensa instruction)".to_string(),
                }
            }
            MachineFamily::CortexM => {
                let pc = pc & !1;
                match machine.bus.read_u16(pc as u64) {
                    Ok(h1) => {
                        let is_32bit = (h1 & 0xE000) == 0xE000 && (h1 & 0x1800) != 0;
                        if is_32bit {
                            match machine.bus.read_u16(pc as u64 + 2) {
                                Ok(h2) => format!("{:?}", decode_thumb_32(h1, h2)),
                                Err(_) => "?? (Error reading h2)".to_string(),
                            }
                        } else {
                            format!("{:?}", decode_thumb_16(h1))
                        }
                    }
                    Err(_) => "?? (Error reading h1)".to_string(),
                }
            }
            // No shared AVR decoder in the wasm Trace panel yet — show the raw
            // opcode word so the pane is never empty / wrong-arch.
            MachineFamily::Avr => match machine.bus.read_u16(pc as u64) {
                Ok(word) => format!("AVR {word:#06x}"),
                Err(_) => "?? (Error reading AVR instruction)".to_string(),
            },
        }
    }

    /// Execute up to max_cycles steps, returning the number actually executed.
    #[wasm_bindgen]
    pub fn step_batch(&mut self, max_cycles: u32) -> Result<u32, JsValue> {
        let machine = self.machine();
        let before = machine.total_cycles;
        match machine.advance(AdvanceRequest::run(Some(u64::from(max_cycles)))) {
            Ok(report) => {
                let elapsed = machine.total_cycles.saturating_sub(before);
                debug_assert_eq!(elapsed, report.elapsed_cycles);
                Ok(elapsed.min(u64::from(u32::MAX)) as u32)
            }
            Err(e) => {
                let elapsed = machine.total_cycles.saturating_sub(before);
                let executed = elapsed.min(u64::from(u32::MAX)) as u32;
                if executed > 0 {
                    Ok(executed)
                } else {
                    Err(JsValue::from_str(&format!("Step Error: {}", e)))
                }
            }
        }
    }

    /// Execute one measured batch and return both wall-clock timing and core
    /// run-loop counters. Intended for worker/Playwright profiling; normal
    /// animation still calls `step_batch`.
    #[wasm_bindgen]
    pub fn step_batch_profile(&mut self, max_cycles: u32) -> Result<JsValue, JsValue> {
        let t0 = perf_now();
        let machine = self.machine();
        let before = machine.total_cycles;
        machine.reset_step_profile();
        let advance_result = machine.advance(AdvanceRequest::run(Some(u64::from(max_cycles))));
        let elapsed = machine.total_cycles.saturating_sub(before);
        let executed = match advance_result {
            Ok(report) => {
                debug_assert_eq!(elapsed, report.elapsed_cycles);
                report.elapsed_cycles.min(u64::from(u32::MAX)) as u32
            }
            Err(e) => {
                let partial = elapsed.min(u64::from(u32::MAX)) as u32;
                if partial == 0 {
                    return Err(JsValue::from_str(&format!("Step Error: {}", e)));
                }
                partial
            }
        };
        let profile = machine.step_profile();
        let t1 = perf_now();

        serde_wasm_bindgen::to_value(&WasmStepBatchProfile {
            requested_cycles: max_cycles,
            executed_cycles: executed,
            wall_ms: t1 - t0,
            cycles_per_second: if t1 > t0 {
                (executed as f64) * 1000.0 / (t1 - t0)
            } else {
                0.0
            },
            cpu_instructions: profile.cpu_instructions,
            cpu_batches: profile.cpu_batches,
            peripheral_ticks: profile.peripheral_ticks,
            peripheral_ticked_entries: profile.peripheral_ticked_entries,
            bus_tick_entries: profile.bus_tick_entries,
            legacy_tick_entries: profile.legacy_tick_entries,
        })
        .map_err(|e| JsValue::from_str(&format!("profile serialize: {e}")))
    }

    // ──────────────────────────────────────────────────────────────────────
    //  IO-Link DI demo: 74HC165 input toggling + IO-Link master readout.
    //  These find the device by iterating the bus (the shifter/master are
    //  `external_devices`, not `board_io` bindings), which suits the single
    //  shifter + single master of the IO-Link DI/DO demo.
    // ──────────────────────────────────────────────────────────────────────

    // DEPRECATED: renamed to install_esp32_arduino_quirks for clarity.
    // The concern is Arduino-ESP32 firmware bootstrap (heap-caps thunks,
    // dual-core handshake fakery, sendHello stub, WifiWsLink::loop stub,
    // esp_crc8 thunk, etc.), not a specific customer product. Kept as a
    // thin wrapper so the standalone /playground page (and any other
    // pre-rename caller) keeps working.
    #[wasm_bindgen]
    #[allow(deprecated)]
    #[deprecated(
        note = "Renamed to install_esp32_arduino_quirks — the bootstrap is generic Arduino-ESP32 glue, not firmware-specific."
    )]
    /// #124 Phase 4: enable/disable the browser-side JIT fast-path. When
    /// on, `step_with_esp32_aids` short-circuits any pre-fetch step
    /// whose PC matches the JIT'd hot block (`0x400829cc`) into a wasm
    /// call constructed via `js_sys::WebAssembly`. Off by default —
    /// callers opt in from JS once they've benchmarked.
    #[wasm_bindgen]
    /// Wall-clock attribution for the open profiling window, as text, with this
    /// chip's peripheral names resolved.
    ///
    /// The window is per-THREAD, not per-simulator: on a multi-chip lab every
    /// chip in this worker records into it, and the report says so.
    pub fn profile_report(&mut self) -> String {
        self.machine().profile_report().render()
    }

    /// The same attribution as JSON, for a HUD to render.
    pub fn profile_report_json(&mut self) -> String {
        let report = self.machine().profile_report();
        let rows: Vec<serde_json::Value> = report
            .rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "ns": r.ns,
                    "calls": r.calls,
                    "percent": r.percent,
                })
            })
            .collect();
        serde_json::json!({
            "clock": format!("{:?}", report.clock),
            "windowNs": report.window_ns,
            "machines": report.machines,
            "unattributedNs": report.unattributed_ns,
            "rows": rows,
        })
        .to_string()
    }

    pub fn set_jit_enabled(&mut self, enabled: bool) {
        self.jit_browser_enabled = enabled;
        if !enabled {
            // Cleanly drop the cached module + closures so the next
            // enable rebuilds from scratch.
            self.jit_browser_cache = None;
        }
    }

    /// Enable/disable scheduler-safe CPU idle fast-forwarding. Off by default;
    /// browser callers opt in explicitly after comparing accelerated and
    /// non-accelerated traces for the target firmware.
    #[wasm_bindgen]
    pub fn set_idle_fast_forward_enabled(&mut self, enabled: bool) {
        self.machine().config.idle_fast_forward_enabled = enabled;
    }

    /// Cumulative cycles advanced by idle fast-forward (WFI skip), not
    /// interpreted. Browser `?perf=1` uses this to prove FF is firing; stays
    /// 0 when FF is off or firmware never parks in a skippable idle.
    #[wasm_bindgen]
    pub fn idle_fast_forward_cycles_skipped(&self) -> u64 {
        self.machine
            .as_ref()
            .map(|m| m.idle_fast_forward_cycles_skipped)
            .unwrap_or(0)
    }

    /// Set the peripheral tick interval used by `Machine::run`.
    ///
    /// `1` is the exact default: tick orchestration runs after every executed
    /// instruction. Larger values are a bounded browser acceleration knob for
    /// firmware bring-up paths whose active peripherals are scheduler-driven or
    /// inactive.
    ///
    /// The machine and bus each hold a `SimulationConfig`; both are updated —
    /// the run loop paces ticks off the machine's copy while the legacy-walk
    /// quantum (`tick_elapsed(interval)`) and the HC-SR04 event-scheduling
    /// gate read the bus's, and they must agree or walked peripherals run
    /// `interval`× slow.
    #[wasm_bindgen]
    pub fn set_peripheral_tick_interval(&mut self, interval: u32) {
        let machine = self.machine();
        machine.config.peripheral_tick_interval = interval.max(1);
        machine.bus.config.peripheral_tick_interval = interval.max(1);
    }

    /// The largest `peripheral_tick_interval` this machine's bus can run at
    /// without losing fidelity (see `SystemBus::max_safe_tick_interval`): a
    /// batching interval when every peripheral is scheduler-driven, `1` when
    /// anything non-relaxable (IO-Link master, a live legacy walk, forced
    /// HC-SR04 legacy path) is present. H5 op-modeling FLASH still clamps
    /// CPU quantum via `requires_cycle_accurate` but does not pin this
    /// interval. The TS side calls this once at engine init and feeds the
    /// answer straight into `set_peripheral_tick_interval`.
    #[wasm_bindgen]
    pub fn recommended_tick_interval(&mut self) -> u32 {
        // Machine-level, not bus-level: a dual-core machine must stay at 1 no
        // matter how relaxable its peripherals are (see
        // `Machine::max_safe_tick_interval` for the SMP deadlock this prevents).
        self.machine().max_safe_tick_interval()
    }

    /// Total number of times the browser JIT has dispatched a
    /// compiled block. Useful for confirming the JIT path actually
    /// fired during a benchmark.
    #[wasm_bindgen]
    pub fn jit_hits(&self) -> u64 {
        self.jit_browser_cache
            .as_ref()
            .map(|c| c.total_hits())
            .unwrap_or(0)
    }

    /// Total number of JIT refusals (host bus errors, JS-side
    /// dispatch failures). Surfaced for the bench harness so it can
    /// distinguish "JIT was tried and rejected" from "JIT was never
    /// hit because PC never reached the block".
    #[wasm_bindgen]
    pub fn jit_refusals(&self) -> u64 {
        self.jit_browser_cache
            .as_ref()
            .map(|c| c.refusals)
            .unwrap_or(0)
    }

    /// Bench runner: execute `cycles` `step_with_esp32_aids` iterations
    /// and return elapsed milliseconds (measured via
    /// `performance.now()`). The caller drives this twice — once with
    /// `set_jit_enabled(false)`, once with `set_jit_enabled(true)` —
    /// and compares the two numbers to quantify JIT speedup.
    ///
    /// Returns a `Result<f64, JsValue>`: the `Err` path bubbles step
    /// errors so the bench harness can show a useful message.
    #[wasm_bindgen]
    pub fn bench_jit(&mut self, cycles: u32) -> Result<f64, JsValue> {
        let t0 = perf_now();
        self.step_with_esp32_aids(cycles)?;
        let t1 = perf_now();
        Ok(t1 - t0)
    }

    /// Step `cycles` cycles with the ESP32-classic IPI bridge active. Each
    /// cycle samples the DPORT FROM_CPU intmatrix mapping and trigger
    /// registers, raises the corresponding INTERRUPT bit, and clears the
    /// trigger so the next write re-edges. The dual-core handshake bytes
    /// are re-applied every 10k cycles (matching the e2e test cadence).
    /// Falls back to plain `step` if `install_esp32_arduino_quirks` hasn't
    /// been called yet.
    ///
    /// Dual-core machines use batched [`AdvanceRequest::run`] (same as
    /// [`Self::step_batch`]) so idle fast-forward can engage while PRO_CPU is
    /// WAITI-parked. The old N× `AdvanceRequest::single` path forced quantum-1
    /// and permanently disabled idle FF for the classic-aids playground path.
    #[wasm_bindgen]
    pub fn step_with_esp32_aids(&mut self, cycles: u32) -> Result<(), JsValue> {
        // Real dual-core: a genuine APP_CPU is attached, so the handshake
        // keep-alive and the FROM_CPU IPI bridge below are unnecessary — the
        // firmware drives the rendezvous itself and Machine::advance delivers
        // the cross-core IPI via the DPORT. Use the batched run path so idle
        // FF / WAITI coalesce work (see PR-I).
        if self
            .machine
            .as_ref()
            .is_some_and(|m| m.cpu_secondary.is_some())
            || self.esp32_ipi.is_none()
        {
            // Batched run (idle FF enabled when configured). Always surface
            // CPU errors — unlike `step_batch`, which can return Ok(partial)
            // after a mid-batch fault.
            self.machine()
                .advance(AdvanceRequest::run(Some(u64::from(cycles))))
                .map(|_| ())
                .map_err(|e| JsValue::from_str(&format!("Step Error: {e}")))
        } else {
            self.step_with_esp32_aids_singlecore_ipi(cycles)
        }
    }

    fn step_with_esp32_aids_singlecore_ipi(&mut self, cycles: u32) -> Result<(), JsValue> {
        if self.esp32_ipi.is_none() {
            return self.step_batch(cycles).map(|_| ());
        }
        for i in 0..cycles {
            {
                let machine = self.machine.as_mut().unwrap();
                let bridge = self.esp32_ipi.as_mut().unwrap();
                if let Ok(v) = machine.bus.read_u32(0x3FF0_0164) {
                    let bit = (v & 0x1F) as u8;
                    if v != 0 && bit < 32 {
                        bridge.from_cpu_bit0 = Some(bit);
                    }
                }
                if let Ok(v) = machine.bus.read_u32(0x3FF0_0168) {
                    let bit = (v & 0x1F) as u8;
                    if v != 0 && bit < 32 {
                        bridge.from_cpu_bit1 = Some(bit);
                    }
                }
                if let Ok(v0) = machine.bus.read_u32(0x3FF0_00DC) {
                    if v0 != 0 && v0 != bridge.last_from_cpu0_val {
                        if let Some(bit) = bridge.from_cpu_bit0 {
                            machine.cpu.raise_interrupt_bits(1u32 << bit);
                        }
                        let _ = machine.bus.write_u32(0x3FF0_00DC, 0);
                    }
                    bridge.last_from_cpu0_val = 0;
                }
                if let Ok(v1) = machine.bus.read_u32(0x3FF0_00E0) {
                    if v1 != 0 && v1 != bridge.last_from_cpu1_val {
                        if let Some(bit) = bridge.from_cpu_bit1 {
                            machine.cpu.raise_interrupt_bits(1u32 << bit);
                        }
                        let _ = machine.bus.write_u32(0x3FF0_00E0, 0);
                    }
                    bridge.last_from_cpu1_val = 0;
                }
                // Dual-core handshake keep-alive. Re-asserts the handshake
                // bytes every 10k cycles so .bss zero-init can't wipe them
                // before the spin-wait check in call_start_cpu0. Uses the
                // per-firmware addresses resolved by autodiscover when
                // available; falls back to the hardcoded reference-firmware
                // addresses for the legacy install_esp32_arduino_quirks
                // path.
                if i % 10_000 == 0 {
                    if !bridge.handshake_bytes.is_empty() {
                        for &addr in &bridge.handshake_bytes {
                            let _ = machine.bus.write_u8(addr as u64, 0x01);
                        }
                    } else {
                        let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_6F01, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_6F02, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_6FFD, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_6FFE, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_7190, 0x01);
                    }
                }
            }

            // #124 Phase 4: browser-side JIT fast-path. Runs BEFORE
            // `machine.step()` so a successful JIT call advances PC
            // past the hot block (0x400829cc -> 0x400829e4) and the
            // regular step picks up at the post-block callx8.
            // CCOUNT advance happens inside the JIT helper to keep
            // CCOMPARE0 edge detection honest.
            if self.jit_browser_enabled {
                let machine = self.machine.as_mut().unwrap();
                if self.jit_browser_cache.is_none() {
                    self.jit_browser_cache = Some(Box::new(jit_browser::BrowserJitCache::new()));
                }
                let cache = self.jit_browser_cache.as_mut().unwrap();
                if let Some(any) = machine.cpu.as_any_mut() {
                    if let Some(xt) = any.downcast_mut::<labwired_core::cpu::XtensaLx7>() {
                        jit_browser::try_browser_jit_step(xt, &mut machine.bus, cache);
                    }
                }
            }

            self.machine()
                .step()
                .map_err(|e| JsValue::from_str(&format!("Step Error: {e}")))?;
        }
        Ok(())
    }
}

// ── Browser-side performance.now() shim ────────────────────────────────
//
// `web-sys` would bring in a large generated binding tree just to call
// `performance.now()`. We use an explicit `wasm-bindgen` import instead.
// Same ABI; ~zero overhead. The matching console.warn shim lives in
// `jit_browser.rs` to keep its module self-contained.

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn perf_now() -> f64;
}

/// Nanosecond clock for [`labwired_core::profile`], from the same
/// `performance.now()` import above.
///
/// ⚠️ **Resolution is much coarser than the spans being timed.** Chrome clamps
/// `performance.now()` to 100 µs outside a cross-origin-isolated context (5 µs
/// inside one), while a single peripheral event handler runs in ~100 ns. Any
/// INDIVIDUAL event therefore measures 0 or one whole clamp step — the per-call
/// numbers are noise.
///
/// The SUMS are still sound: truncation against a clock whose phase is
/// uncorrelated with the work is unbiased, so over the millions of events in a
/// real window the totals converge on the truth. Read the browser report as
/// subsystem shares over a long window, never as the cost of one call, and
/// sanity-check it against the `unattributed` row.
fn profile_now_ns() -> u64 {
    (perf_now() * 1_000_000.0) as u64
}

/// Start an engine profiling window in the browser, installing the
/// `performance.now()` clock. Without this the wasm build has no clock at all
/// and every duration would read zero — see `labwired_core::profile`.
#[wasm_bindgen]
pub fn profile_start() {
    labwired_core::profile::set_clock(profile_now_ns);
    labwired_core::profile::start();
}

/// Close the profiling window. The report survives until the next
/// [`profile_start`].
#[wasm_bindgen]
pub fn profile_stop() {
    labwired_core::profile::stop();
}

/// Is the engine profiler recording?
#[wasm_bindgen]
pub fn profile_enabled() -> bool {
    labwired_core::profile::enabled()
}

/// A shared UART cross-link medium, owned by the host. Create one per multi-chip
/// lab-group and pass it to every chip's `attach_uart_wire`; chips sharing a bus
/// exchange bytes, chips on different buses are isolated. A fresh `WireBus` per
/// lab (re)load replaces the former module-global reset — a new bus starts empty,
/// so no stale link buffers can leak into the new station.
#[wasm_bindgen]
pub struct WireBus {
    inner: labwired_core::network::virtual_uart_wire::VirtualWireBus,
}

#[wasm_bindgen]
impl WireBus {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> WireBus {
        WireBus {
            inner: labwired_core::network::virtual_uart_wire::VirtualWireBus::new(),
        }
    }

    /// Drop every link's buffered bytes on this bus. Rarely needed — prefer a
    /// fresh `WireBus` per lab load — but exposed for in-place resets.
    #[wasm_bindgen]
    pub fn clear(&self) {
        self.inner.clear();
    }
}

/// Shared lab air: nRF `VirtualAirBus` + ESP `BleAirBus` +
/// [`SimMqttFabric`] + optional path-loss [`RfMedium`]. Create ONE per
/// lab-group and pass it to every chip via `attach_lab_air` — same pattern as
/// [`WireBus`]. Path-loss CSQ and MQTT fabric share this air.
#[wasm_bindgen]
pub struct AirBus {
    nrf: labwired_core::peripherals::nrf52::radio::VirtualAirBus,
    ble: labwired_core::peripherals::ble_air::BleAirBus,
    cellular: labwired_core::network::SimMqttFabric,
}

#[wasm_bindgen]
impl AirBus {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> AirBus {
        AirBus {
            nrf: labwired_core::peripherals::nrf52::radio::VirtualAirBus::new(),
            ble: labwired_core::peripherals::ble_air::BleAirBus::new(),
            cellular: labwired_core::network::SimMqttFabric::new(),
        }
    }

    /// Enable path-loss medium (seeded). Positions via `set_node_position`.
    /// Co-located nodes stay lossless until placed apart.
    #[wasm_bindgen]
    pub fn enable_path_loss(&self, seed: f64, rssi_floor_dbm: f64) {
        use labwired_core::peripherals::rf_medium::{PathLossParams, RfMedium};
        let mut params = PathLossParams::default();
        if rssi_floor_dbm.is_finite() {
            params.rssi_floor_dbm = rssi_floor_dbm;
        }
        let seed_u = if seed.is_finite() && seed >= 0.0 {
            seed as u64
        } else {
            0
        };
        self.nrf
            .attach_medium(RfMedium::new(seed_u).with_params(params));
    }

    /// Place a node (MCU part id) in metres for path-loss.
    #[wasm_bindgen]
    pub fn set_node_position(&self, node_id: &str, x: f64, y: f64) {
        use labwired_core::peripherals::rf_medium::NodePosition;
        self.nrf.set_node_position(node_id, NodePosition { x, y });
    }

    #[wasm_bindgen]
    pub fn clear_nrf(&self) {
        self.nrf.clear();
    }

    #[wasm_bindgen]
    pub fn clear_ble(&self) {
        self.ble.clear();
    }

    /// Drop SimMqttFabric state (publish log + subscriptions).
    #[wasm_bindgen]
    pub fn mqtt_fabric_clear(&self) {
        self.cellular.clear();
    }

    /// True if any modem on this air published to `topic` (exact match).
    #[wasm_bindgen]
    pub fn mqtt_fabric_has_publish(&self, topic: &str) -> bool {
        self.cellular.has_publish_on(topic)
    }

    /// Latest payload bytes for an exact topic, or empty if none.
    #[wasm_bindgen]
    pub fn mqtt_fabric_last_payload(&self, topic: &str) -> Vec<u8> {
        self.cellular.last_payload_on(topic).unwrap_or_default()
    }

    /// Inspect fabric: up to `limit` lines of `topic\\tpayload` (most recent first).
    #[wasm_bindgen]
    pub fn mqtt_fabric_inspect(&self, limit: f64) -> String {
        let n = if limit.is_finite() && limit > 0.0 {
            limit as usize
        } else {
            16
        };
        self.cellular.inspect_lines(n.min(64)).join("\n")
    }

    // --- deprecated aliases (wasm keeps old names working one release) ---
    #[wasm_bindgen]
    pub fn clear_cellular(&self) {
        self.mqtt_fabric_clear();
    }
    #[wasm_bindgen]
    pub fn cellular_has_publish(&self, topic: &str) -> bool {
        self.mqtt_fabric_has_publish(topic)
    }
    #[wasm_bindgen]
    pub fn cellular_last_payload(&self, topic: &str) -> Vec<u8> {
        self.mqtt_fabric_last_payload(topic)
    }
    #[wasm_bindgen]
    pub fn cellular_inspect(&self, limit: f64) -> String {
        self.mqtt_fabric_inspect(limit)
    }
}

/// Parse a JS `{ name: Uint8Array }` object into a `name → bytes` map. Values
/// that aren't `Uint8Array` are skipped; `null`/`undefined` → empty map.
///
/// This is the generic on-demand binary-blob channel: a board fetches only the
/// assets it needs (e.g. the ESP32-S3 boot ROM) and passes them through
/// `new_from_config`, so no per-board blob is baked into the shared wasm bundle.
/// Size the ESP32-S3 flash backing for a merged-image (`--rom-boot`) run.
///
/// The chip descriptor is the authority — the part's capacity is a property of
/// the module, not of how much of it this particular build fills. The model
/// publishes that capacity as the JEDEC RDID capacity byte
/// (`peripherals/esp32s3/spi_mem_flash.rs`, `log2(backing.len())`), and
/// `esp_flash` compares it against the size in the app image header and aborts
/// the boot on a mismatch. Deriving the backing from the image length instead
/// made an 8,455,860-byte N16R8 image publish an 8 MiB part against its own
/// 16 MB header. The image length is only a floor, so a chip YAML that
/// understates the part cannot truncate the image itself.
fn esp32s3_flash_backing_size(chip_flash_size: u64, image_len: usize) -> u32 {
    let declared = u32::try_from(chip_flash_size).unwrap_or(u32::MAX);
    let image = u32::try_from(image_len).unwrap_or(u32::MAX);
    declared.max(image).max(4 * 1024 * 1024)
}

fn parse_named_blobs(blobs: &JsValue) -> std::collections::HashMap<String, Vec<u8>> {
    use wasm_bindgen::JsCast;
    let mut map = std::collections::HashMap::new();
    if blobs.is_undefined() || blobs.is_null() {
        return map;
    }
    if let Ok(obj) = blobs.clone().dyn_into::<js_sys::Object>() {
        for entry in js_sys::Object::entries(&obj).iter() {
            if let Ok(pair) = entry.dyn_into::<js_sys::Array>() {
                if let (Some(key), Ok(arr)) = (
                    pair.get(0).as_string(),
                    pair.get(1).dyn_into::<js_sys::Uint8Array>(),
                ) {
                    map.insert(key, arr.to_vec());
                }
            }
        }
    }
    map
}

// WasmGdbEventLoop removed — see `gdb_process_packet` above for the rationale.
// Restoring this requires `LabwiredTarget` to be implemented for an arch-erased
// CPU type, which is the follow-up tracked alongside Phase 1.

#[cfg(all(test, not(target_arch = "wasm32")))]
mod esp32s3_flash_backing_tests {
    use super::esp32s3_flash_backing_size;

    /// The regression this guards: an ESP-IDF N16R8 image that does not fill
    /// its 16 MiB part. Sizing the backing from the image made RDID report an
    /// 8 MiB chip and the app aborted with
    /// "Detected size(8192k) smaller than the size in the binary image
    /// header(16384k). Probe failed." — before `app_main` ever ran.
    #[test]
    fn n16r8_image_smaller_than_the_part_still_gets_the_parts_capacity() {
        // configs/chips/esp32s3.yaml declares 16384KB.
        let chip = 16 * 1024 * 1024;
        // The Doom merged image: bootloader + partition table + app + a 4 MB
        // WAD at 0x410000 = 8,455,860 bytes.
        assert_eq!(esp32s3_flash_backing_size(chip, 8_455_860), 16 * 1024 * 1024);
    }

    /// A chip YAML that understates the part must not truncate a bigger image.
    #[test]
    fn image_longer_than_the_declared_part_is_never_truncated() {
        assert_eq!(
            esp32s3_flash_backing_size(4 * 1024 * 1024, 8_455_860),
            8_455_860
        );
    }

    /// A chip descriptor with no usable flash size still gets the 4 MiB floor.
    #[test]
    fn missing_chip_flash_size_falls_back_to_the_four_mib_floor() {
        assert_eq!(esp32s3_flash_backing_size(0, 0), 4 * 1024 * 1024);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod machine_advance_tests {
    use super::*;
    use std::collections::BTreeSet;

    fn wrap_test_machine<C: Cpu + 'static>(
        cpu: C,
        mut bus: SystemBus,
        arch: MachineFamily,
    ) -> WasmSimulator {
        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(uart_sink.clone(), false);
        let uart_rx_bufs = bus.attach_uart_rx_source();
        let cpu: Box<dyn Cpu> = Box::new(cpu);
        let mut machine = Machine::new(cpu, bus);
        machine.config.peripheral_tick_interval = 64;
        machine.bus.config.peripheral_tick_interval = 64;

        WasmSimulator {
            machine: Some(machine),
            board_io: Vec::new(),
            uart_sink,
            console: ConsoleCapture::new(HostConsole::Undeclared, HostConsole::UsbSerialJtag),
            uart_rx_bufs,
            arch,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        }
    }

    fn arm_simulator() -> WasmSimulator {
        let mut bus = SystemBus::new();
        let mut cpu = labwired_core::cpu::CortexM::new();
        for index in 0..64_u64 {
            bus.write_u16(index * 2, 0xBF00).unwrap();
        }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, MachineFamily::CortexM)
    }

    fn configured_arm_simulator() -> WasmSimulator {
        let mut bus = SystemBus::new();
        let (mut cpu, _) = configure_cortex_m(&mut bus);
        for index in 0..64_u64 {
            bus.write_u16(index * 2, 0xBF00).unwrap();
        }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, MachineFamily::CortexM)
    }

    fn riscv_simulator() -> WasmSimulator {
        let mut bus = SystemBus::new();
        let mut cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
        for index in 0..64_u64 {
            bus.write_u32(index * 4, 0x0000_0013).unwrap();
        }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, MachineFamily::RiscV)
    }

    fn xtensa_simulator() -> WasmSimulator {
        let mut bus = SystemBus::new();
        let mut cpu = labwired_core::cpu::XtensaLx7::new();
        for index in 0..64_u64 {
            bus.write_u8(index * 2, 0x3d).unwrap();
            bus.write_u8(index * 2 + 1, 0xf0).unwrap();
        }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, MachineFamily::Xtensa)
    }

    fn assert_batch_matches_32_singles(
        build: impl Fn() -> WasmSimulator,
        expected_batch_count: u64,
        expect_peripherals: bool,
    ) {
        let mut singles = build();
        let mut batch = build();

        for _ in 0..32 {
            singles.step_single().expect("single step");
        }
        assert_eq!(batch.step_batch(32).expect("batch step"), 32);

        let singles = singles.machine.as_ref().unwrap();
        let batch = batch.machine.as_ref().unwrap();
        let singles_snapshot = singles.snapshot();
        let batch_snapshot = batch.snapshot();

        assert_eq!(
            serde_json::to_value(&singles_snapshot).unwrap(),
            serde_json::to_value(&batch_snapshot).unwrap()
        );
        assert_eq!(
            serde_json::to_value(singles.cpu.snapshot()).unwrap(),
            serde_json::to_value(batch.cpu.snapshot()).unwrap()
        );
        assert_eq!(singles_snapshot.peripherals, batch_snapshot.peripherals);
        if expect_peripherals {
            assert!(!singles_snapshot.peripherals.is_empty());
            assert!(!batch_snapshot.peripherals.is_empty());
        }
        assert_eq!(singles.total_cycles, batch.total_cycles);
        assert_eq!(singles.bus.current_cycle, batch.bus.current_cycle);
        assert_eq!(singles.cpu.get_pc(), batch.cpu.get_pc());
        assert_ne!(singles.cpu.get_pc(), 0);
        assert_eq!((singles.total_cycles, batch.total_cycles), (32, 32));

        let singles_profile = singles.step_profile();
        let batch_profile = batch.step_profile();
        assert_eq!(singles_profile.cpu_instructions, 32);
        assert_eq!(batch_profile.cpu_instructions, 32);
        assert_eq!(singles_profile.cpu_batches, 32);
        assert_eq!(batch_profile.cpu_batches, expected_batch_count);
        if expected_batch_count == 1 {
            assert!(batch_profile.cpu_batches < batch_profile.cpu_instructions);
        }

        // CPU batch count is intentionally execution-path dependent. Every
        // peripheral-work counter must remain identical across the two paths.
        assert_eq!(
            singles_profile.peripheral_ticks,
            batch_profile.peripheral_ticks
        );
        assert_eq!(
            singles_profile.peripheral_ticked_entries,
            batch_profile.peripheral_ticked_entries
        );
        assert_eq!(
            singles_profile.bus_tick_entries,
            batch_profile.bus_tick_entries
        );
        assert_eq!(
            singles_profile.legacy_tick_entries,
            batch_profile.legacy_tick_entries
        );
    }

    #[test]
    fn arm_batch_matches_32_single_boundaries() {
        assert_batch_matches_32_singles(arm_simulator, 1, false);
    }

    #[test]
    fn configured_arm_batch_matches_32_single_boundaries() {
        // A real Cortex-M topology contains an SCB, and its reset-fidelity rail
        // used to commit one instruction per CPU batch for the life of the bus
        // — so this case expected 32 batches where every other arch expected 1.
        // The rail is now the latch the SCB shares with the core, which cuts
        // the batch only on the instruction that actually writes AIRCR, so a
        // configured Cortex-M batches like everything else.
        //
        // The 32-vs-1 batch count is the ONLY thing that changed. Everything
        // this helper asserts before reaching the count — full machine
        // snapshot, CPU snapshot, peripheral list, total_cycles,
        // bus.current_cycle, PC, and every peripheral-work counter — is still
        // identical between 32 single boundaries and one 32-instruction batch,
        // on a real topology WITH peripherals attached (`expect_peripherals`).
        // That equivalence is the fidelity claim behind the whole change.
        assert_batch_matches_32_singles(configured_arm_simulator, 1, true);
    }

    #[test]
    fn riscv_batch_matches_32_single_boundaries() {
        assert_batch_matches_32_singles(riscv_simulator, 1, false);
    }

    #[test]
    fn xtensa_batch_matches_32_single_boundaries() {
        assert_batch_matches_32_singles(xtensa_simulator, 1, false);
    }

    #[test]
    fn step_batch_profile_schema_is_exact() {
        let value = serde_json::to_value(WasmStepBatchProfile {
            requested_cycles: 1,
            executed_cycles: 2,
            wall_ms: 3.0,
            cycles_per_second: 4.0,
            cpu_instructions: 5,
            cpu_batches: 6,
            peripheral_ticks: 7,
            peripheral_ticked_entries: 8,
            bus_tick_entries: 9,
            legacy_tick_entries: 10,
        })
        .unwrap();
        let actual = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let expected = [
            "bus_tick_entries",
            "cpu_batches",
            "cpu_instructions",
            "cycles_per_second",
            "executed_cycles",
            "legacy_tick_entries",
            "peripheral_ticked_entries",
            "peripheral_ticks",
            "requested_cycles",
            "wall_ms",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();

        assert_eq!(actual, expected);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod romboot_tests {
    //! Regression guard for the ESP32-C3 wasm faithful ROM-boot path.
    //!
    //! Exercises the exact browser entry [`WasmSimulator::new_from_config_riscv_romboot`]
    //! on the native test target (a real headless browser isn't available): it
    //! provisions the boot ROM from the two ROM blobs, injects them into the
    //! chip's `rom`/`rom_data` regions, hands the merged flash image to the
    //! shared core builder, resets to `0x4000_0000`, and runs the genuine mask
    //! ROM → 2nd-stage bootloader → `app_main()`. Asserts it reaches the IDF
    //! `Calling app_main()` / "Hello world!" banner. Zero thunks.
    //!
    //! `#[ignore]` because the faithful path spends ~150M steps in the real ROM
    //! before `app_main`; run it in release:
    //!   `cargo test -p labwired-wasm --release romboot -- --ignored --nocapture`
    use super::*;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn parses_esp32c3_app_segments_from_merged_flash() {
        let mut flash = vec![0xff; ESP32C3_APP_IMAGE_OFFSET + 256];
        let app = ESP32C3_APP_IMAGE_OFFSET;
        flash[app] = ESP_IMAGE_MAGIC;
        flash[app + 1] = 2;
        flash[app + 4..app + 8].copy_from_slice(&0x4200_1234u32.to_le_bytes());

        let mut cursor = app + ESP_IMAGE_HEADER_LEN;
        flash[cursor..cursor + 4].copy_from_slice(&0x3FC8_0010u32.to_le_bytes());
        flash[cursor + 4..cursor + 8].copy_from_slice(&3u32.to_le_bytes());
        cursor += 8;
        flash[cursor..cursor + 3].copy_from_slice(&[1, 2, 3]);
        cursor += 3;

        flash[cursor..cursor + 4].copy_from_slice(&0x4200_2000u32.to_le_bytes());
        flash[cursor + 4..cursor + 8].copy_from_slice(&4u32.to_le_bytes());
        cursor += 8;
        flash[cursor..cursor + 4].copy_from_slice(&[4, 5, 6, 7]);

        let image = esp32c3_app_program_image_from_merged_flash(&flash).expect("parse app image");

        assert_eq!(image.entry_point, 0x4200_1234);
        assert_eq!(image.arch, CoreArch::RiscV);
        assert_eq!(image.segments.len(), 2);
        assert_eq!(image.segments[0].start_addr, 0x3FC8_0010);
        assert_eq!(image.segments[0].data, vec![1, 2, 3]);
        assert_eq!(image.segments[1].start_addr, 0x4200_2000);
        assert_eq!(image.segments[1].data, vec![4, 5, 6, 7]);
    }

    #[test]
    fn rejects_flash_without_esp_app_magic_at_app_offset() {
        let flash = vec![0xff; ESP32C3_APP_IMAGE_OFFSET + ESP_IMAGE_HEADER_LEN];

        let err = esp32c3_app_program_image_from_merged_flash(&flash).unwrap_err();

        assert!(err.contains("bad magic"), "{err}");
    }

    #[test]
    fn parses_esp32c3_bootloader_segments_from_merged_flash() {
        let mut flash = vec![0xff; 128];
        flash[0] = ESP_IMAGE_MAGIC;
        flash[1] = 1;
        flash[4..8].copy_from_slice(&0x4038_0100u32.to_le_bytes());

        let cursor = ESP_IMAGE_HEADER_LEN;
        flash[cursor..cursor + 4].copy_from_slice(&0x4038_0100u32.to_le_bytes());
        flash[cursor + 4..cursor + 8].copy_from_slice(&4u32.to_le_bytes());
        flash[cursor + 8..cursor + 12].copy_from_slice(&[0x13, 0x00, 0x00, 0x00]);

        let image =
            esp32c3_bootloader_program_image_from_merged_flash(&flash).expect("parse bootloader");

        assert_eq!(image.entry_point, 0x4038_0100);
        assert_eq!(image.segments.len(), 1);
        assert_eq!(image.segments[0].start_addr, 0x4038_0100);
        assert_eq!(image.segments[0].data, vec![0x13, 0x00, 0x00, 0x00]);
    }

    #[test]
    #[ignore = "boots the real C3 mask ROM (~150M steps); run with --release --ignored"]
    fn wasm_romboot_reaches_app_main() {
        let manifest_dir = root();
        let chip_yaml =
            std::fs::read_to_string(manifest_dir.join("../../configs/chips/esp32c3.yaml"))
                .expect("read esp32c3 chip yaml");
        let system_yaml =
            std::fs::read_to_string(manifest_dir.join("../../configs/systems/esp32c3-devkit.yaml"))
                .expect("read esp32c3-devkit system yaml");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip yaml");
        let manifest: SystemManifest =
            serde_yaml::from_str(&system_yaml).expect("parse system yaml");

        // The browser fetches these on demand; here we read the vendored ROM
        // bins + the committed IDF hello_world flash image directly.
        let irom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_rom.bin"))
            .expect("read vendored C3 IROM");
        let drom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_drom.bin"))
            .expect("read vendored C3 DROM");
        let flash =
            std::fs::read(manifest_dir.join("tests/fixtures/esp32c3-hello-world-flash.bin"))
                .expect("read C3 hello_world flash image");

        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        blobs.insert("esp32c3_irom".into(), irom);
        blobs.insert("esp32c3_drom".into(), drom);
        blobs.insert("esp32c3_flash".into(), flash);

        let mut sim = WasmSimulator::new_from_config_riscv_romboot(&chip, &manifest, &blobs)
            .expect("construct C3 rom-boot WasmSimulator");

        // Step in batches; stop as soon as the app_main banner appears.
        const BATCH: u32 = 1_000_000;
        const MAX_STEPS: u64 = 300_000_000;
        let mut steps: u64 = 0;
        let mut reached = false;
        while steps < MAX_STEPS {
            sim.step(BATCH).expect("step");
            steps += BATCH as u64;
            let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
            if out.contains("Hello world!") {
                reached = true;
                eprintln!("reached app_main at ~{steps} steps");
                break;
            }
        }
        let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
        assert!(
            reached,
            "C3 wasm rom-boot did not reach app_main within {MAX_STEPS} steps.\n\
             --- captured serial ---\n{out}"
        );
        assert!(
            out.contains("Calling app_main()"),
            "expected IDF 'Calling app_main()' banner; got:\n{out}"
        );
    }

    /// Decisive proof the browser OLED lab paints: boot the curated
    /// `esp32c3-oled-demo` IDF flash image FAITHFULLY through the real mask ROM
    /// (the exact browser entry `new_from_config_riscv_romboot`), let the
    /// firmware's register-level SSD1306 driver run, then read the panel's
    /// GDDRAM back through the same `get_ssd1306_framebuffer` accessor the
    /// playground/embed uses and assert a non-trivial number of pixels are lit.
    ///
    /// Zero thunks: every lit pixel is a byte the firmware pushed via a genuine
    /// I²C transaction the simulated C3 command-list controller executed against
    /// the attached SSD1306 model. No hardcoded PCs, no faked framebuffer.
    ///
    /// `#[ignore]` for the same reason as the app_main guard (~150M ROM steps);
    /// run with:
    ///   `cargo test -p labwired-wasm --release romboot_oled -- --ignored --nocapture`
    #[test]
    #[ignore = "boots the real C3 mask ROM then paints the OLED; run with --release --ignored"]
    fn wasm_romboot_oled_paints() {
        let manifest_dir = root();
        let chip_yaml =
            std::fs::read_to_string(manifest_dir.join("../../configs/chips/esp32c3.yaml"))
                .expect("read esp32c3 chip yaml");
        let system_yaml = std::fs::read_to_string(
            manifest_dir.join("../../configs/systems/esp32c3-oled-demo.yaml"),
        )
        .expect("read esp32c3-oled-demo system yaml");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip yaml");
        let manifest: SystemManifest =
            serde_yaml::from_str(&system_yaml).expect("parse system yaml");

        let irom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_rom.bin"))
            .expect("read vendored C3 IROM");
        let drom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_drom.bin"))
            .expect("read vendored C3 DROM");
        let flash = std::fs::read(manifest_dir.join("tests/fixtures/esp32c3-oled-demo-flash.bin"))
            .expect("read C3 OLED demo flash image");

        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        blobs.insert("esp32c3_irom".into(), irom);
        blobs.insert("esp32c3_drom".into(), drom);
        blobs.insert("esp32c3_flash".into(), flash);

        let mut sim = WasmSimulator::new_from_config_riscv_romboot(&chip, &manifest, &blobs)
            .expect("construct C3 rom-boot WasmSimulator");

        // Step until the OLED framebuffer holds a non-trivial picture. The
        // firmware paints once shortly after app_main; poll the same accessor
        // the playground uses.
        const BATCH: u32 = 1_000_000;
        const MAX_STEPS: u64 = 300_000_000;
        // "LabWired" + "OLED LAB C3" + frame + bar lights well over this many.
        const MIN_LIT: usize = 400;
        let mut steps: u64 = 0;
        let mut lit = 0usize;
        let mut painted = false;
        while steps < MAX_STEPS {
            sim.step(BATCH).expect("step");
            steps += BATCH as u64;
            if let Ok(fb) = sim.get_ssd1306_framebuffer("oled") {
                lit = fb.iter().map(|b| b.count_ones() as usize).sum();
                if lit >= MIN_LIT {
                    painted = true;
                    eprintln!("OLED painted: {lit} lit pixels at ~{steps} steps");
                    break;
                }
            }
        }
        let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
        assert!(
            painted,
            "C3 OLED lab did not paint (>= {MIN_LIT} lit pixels) within {MAX_STEPS} steps; \
             last count = {lit}.\n--- captured serial ---\n{out}"
        );
    }

    /// Accelerated C3 flash shares must still run the real app image and paint
    /// attached devices. This skips the mask-ROM replay, but does not fake the
    /// OLED: pixels must come from firmware I2C writes into the SSD1306 model.
    ///
    /// Uses `step_batch` (browser worker path via `Machine::run`), not per-insn
    /// `step`, and applies the same tick + idle-FF policy the playground sets
    /// after `recommended_tick_interval()`.
    #[test]
    #[ignore = "browser-path C3 fast-start paint; run with --release --ignored --nocapture"]
    fn wasm_c3_flash_fast_start_oled_paints_quickly() {
        let (mut sim, rec_tick) = c3_browser_fast_start_sim();
        apply_browser_c3_policy(&mut sim, rec_tick);

        const BATCH: u32 = 2_000_000;
        const MAX_STEPS: u64 = 80_000_000;
        const MIN_LIT: usize = 400;
        let mut steps: u64 = 0;
        let mut lit = 0usize;
        let t0 = std::time::Instant::now();
        while steps < MAX_STEPS {
            let n = sim.step_batch(BATCH).expect("step_batch");
            assert!(n > 0, "step_batch returned 0 executed cycles (MCU stuck?)");
            steps += n as u64;
            if let Ok(fb) = sim.get_ssd1306_framebuffer("oled") {
                lit = fb.iter().map(|b| b.count_ones() as usize).sum();
                if lit >= MIN_LIT {
                    // Framebuffer path is the browser OLED door
                    // (`display_artifact` → external_devices id "oled"). Serial
                    // can lag one idle-FF window behind the I²C paint on the
                    // fast-start path — drain a few more batches so the app's
                    // "OLED painted" log reaches the UART sink before we assert.
                    let mut out =
                        String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
                    for _ in 0..8 {
                        if out.contains("oled-lab") || out.contains("OLED painted") {
                            break;
                        }
                        let n = sim.step_batch(BATCH).expect("step_batch drain");
                        steps += n as u64;
                        out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
                    }
                    assert!(
                        out.contains("oled-lab") || out.contains("OLED painted"),
                        "C3 flash fast-start painted (lit={lit}) but did not capture app serial; \
                         captured serial:\n{out}"
                    );
                    eprintln!(
                        "browser-path OLED painted: lit={lit} device_cycles={steps} \
                         rec_tick={rec_tick} wall={:.2}s",
                        t0.elapsed().as_secs_f64()
                    );
                    return;
                }
            }
        }

        let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
        panic!(
            "C3 flash fast-start did not paint OLED (>= {MIN_LIT} lit pixels) within \
             {MAX_STEPS} steps; last count = {lit}.\n--- captured serial ---\n{out}"
        );
    }

    /// Pre-deploy gate: browser C3 path must stay healthy for **several
    /// device-seconds** after paint (no hang, cycles advance, framebuffer
    /// stays lit, serial remains readable). Mirrors worker `step_batch` +
    /// tick-512 + idle FF — not the slow per-instruction `step` API.
    #[test]
    #[ignore = "multi-second browser-path smoke; run with --release --ignored --nocapture"]
    fn wasm_c3_browser_path_runs_few_device_seconds() {
        let (mut sim, rec_tick) = c3_browser_fast_start_sim();
        apply_browser_c3_policy(&mut sim, rec_tick);
        assert!(
            rec_tick >= 64,
            "walk-free C3 should recommend a batched tick interval, got {rec_tick}"
        );

        // 160 MHz silicon: 3 device-seconds ≈ 480e6 cycles. With idle FF the
        // host wall is much shorter; without it this is still a hard progress
        // + stability gate.
        const DEVICE_SECONDS: f64 = 3.0;
        const CPU_HZ: f64 = 160_000_000.0;
        const TARGET_CYCLES: u64 = (DEVICE_SECONDS * CPU_HZ) as u64;
        const BATCH: u32 = 4_000_000;
        const MIN_LIT: usize = 400;

        let t0 = std::time::Instant::now();
        let mut total: u64 = 0;
        let mut lit_at_paint = 0usize;
        let mut painted = false;

        while total < TARGET_CYCLES {
            let n = sim
                .step_batch(BATCH)
                .unwrap_or_else(|e| panic!("step_batch failed at cycle {total}: {e:?}"));
            assert!(
                n > 0,
                "step_batch executed 0 cycles at total={total} — MCU not advancing"
            );
            total += n as u64;

            if let Ok(fb) = sim.get_ssd1306_framebuffer("oled") {
                let lit = fb.iter().map(|b| b.count_ones() as usize).sum::<usize>();
                if !painted && lit >= MIN_LIT {
                    painted = true;
                    lit_at_paint = lit;
                    eprintln!(
                        "paint @ device_cycles={total} lit={lit} wall={:.2}s",
                        t0.elapsed().as_secs_f64()
                    );
                }
                // After paint, framebuffer must not collapse to empty.
                if painted {
                    assert!(
                        lit >= MIN_LIT / 4,
                        "framebuffer collapsed after paint: lit={lit} at cycle {total}"
                    );
                }
            }
        }

        let wall = t0.elapsed().as_secs_f64();
        let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
        let skipped = sim.idle_fast_forward_cycles_skipped();
        let mips = total as f64 / wall / 1.0e6;
        eprintln!(
            "browser-path multi-second: device_cycles={total} (~{DEVICE_SECONDS}s @ 160MHz) \
             wall={wall:.2}s host_MIPS={mips:.1} rec_tick={rec_tick} idle_ff_skipped={skipped} \
             painted={painted} lit_at_paint={lit_at_paint}"
        );
        eprintln!(
            "serial tail (last 800 chars):\n{}",
            out.chars()
                .rev()
                .take(800)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );

        assert!(
            painted,
            "must paint OLED within {DEVICE_SECONDS} device-seconds; serial:\n{out}"
        );
        assert!(
            total >= TARGET_CYCLES,
            "must advance at least {TARGET_CYCLES} device cycles"
        );
        // Sanity: host must make progress (not deadlocked / near-zero MIPS).
        assert!(
            mips > 1.0,
            "host throughput too low ({mips:.3} MIPS) — web MCU effectively not running"
        );
    }

    // Regression guard for the prod bug where the browser's C3 flash-fast-start
    // boot path never attached the WiFi medium (only the slow rom-boot path did),
    // so `WiFi.begin()` timed out on app.labwired.com while the CLI (rom-boot)
    // associated. Drives the EXACT browser path — fast-start ctor + wifi_ap
    // manifest + recommended (512) tick interval + idle fast-forward — against a
    // real Arduino WiFi flash (esp32c3-wifi-stats-flash.bin, the LBC3.1 sketch
    // built with pio). Must reach STA CONNECTED, exercising the real 802.11 →
    // DHCP association through the modeled AP (no thunks).
    #[test]
    fn browser_c3_fast_start_wifi_associates() {
        // Hermetic body: live API numbers move; this gate pins the long JSON
        // that exercises the UART TX-FIFO path (165-byte body → PANEL UPDATED).
        labwired_core::peripherals::esp32c3::virtual_wifi::set_public_stats_body(Some(
            br#"{"generated_at":"2026-07-24T19:39:15.804Z","window_days":90,"boards_supported":9,"parts_supported":82,"labs_opened":69,"simulations_run":3200,"active_sessions":4900}"#
                .to_vec(),
        ));
        struct ClearStats;
        impl Drop for ClearStats {
            fn drop(&mut self) {
                labwired_core::peripherals::esp32c3::virtual_wifi::set_public_stats_body(None);
            }
        }
        let _clear = ClearStats;

        let manifest_dir = root();
        let flash_path = manifest_dir.join("tests/fixtures/esp32c3-wifi-stats-flash.bin");
        let chip: ChipDescriptor = serde_yaml::from_str(
            &std::fs::read_to_string(manifest_dir.join("../../configs/chips/esp32c3.yaml"))
                .expect("chip yaml"),
        )
        .expect("parse chip");
        // A system manifest WITH a wifi_ap block — exactly what the playground
        // emits for a diagram carrying a `wifi-ap` component.
        let manifest: SystemManifest = serde_yaml::from_str(
            "name: \"lbc31-wifi\"\nchip: \"esp32c3.yaml\"\nwifi_ap:\n  ssid: \"labwired-ap\"\n  ip: \"192.168.4.1\"\n  serves: \"labwired-stats\"\nexternal_devices: []\nboard_io: []\n",
        )
        .expect("parse system with wifi_ap");

        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        blobs.insert(
            "esp32c3_irom".into(),
            std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_rom.bin")).expect("irom"),
        );
        blobs.insert(
            "esp32c3_drom".into(),
            std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_drom.bin"))
                .expect("drom"),
        );
        blobs.insert(
            "esp32c3_flash".into(),
            std::fs::read(&flash_path).expect("wifi flash"),
        );
        // The marker the playground injects → dispatcher picks the fast-start
        // ctor (the browser default, the path that lacked WiFi attach).
        blobs.insert(crate::ESP32C3_FLASH_FAST_START_BLOB.to_string(), Vec::new());

        let mut sim = WasmSimulator::new_from_config_riscv_flash_fastboot(&chip, &manifest, &blobs)
            .expect("build fast-start C3 sim");
        let rec = sim.recommended_tick_interval();
        eprintln!("recommended_tick_interval = {rec}");
        apply_browser_c3_policy(&mut sim, rec);

        // Run the whole device pipeline: associate → DHCP → TCP → HTTP fetch of
        // the AP's /v1/public-stats → parse → repaint the e-paper panel. The
        // success line is `PANEL UPDATED` AFTER `PARSED`, not the arrival of
        // the body: stopping at the body is what let the UART-wedge bug ship.
        // The sketch's `HTTP BODY:` line is 165 bytes, longer than the C3's
        // 128-byte TX FIFO, so this only completes if the UART model reports
        // real FIFO occupancy and raises TXFIFO_EMPTY (see
        // `peripherals::esp32c3::uart`). Without that the device wedges here
        // forever with the panel still reading "FETCHING STATS".
        let mut total: u64 = 0;
        let mut fetched = false;
        let mut painted = false;
        while total < 24_000_000_000 {
            let n = sim.step_batch(2_000_000).expect("step");
            if n == 0 {
                break;
            }
            total += u64::from(n);
            let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
            // The AP serves the stats snapshot (boards_supported:9); the LBC3.1
            // sketch logs the fetched JSON body verbatim.
            fetched |= out.contains("boards_supported");
            // The sketch prints PARSED once the body is decoded, then repaints
            // the panel and prints PANEL UPDATED — the third one is the stats
            // paint (boot splash and "FETCHING STATS" are the first two).
            if let Some(parsed_at) = out.find("PARSED boards=") {
                if out[parsed_at..].contains("PANEL UPDATED") {
                    painted = true;
                    break;
                }
            }
            if out.contains("WiFi connect timeout")
                || out.contains("stats fetch failed")
                || out.contains("STATS FETCH FAILED")
            {
                break;
            }
        }
        let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
        eprintln!("--- serial ---\n{out}\n--- end ({total} cycles) ---");
        assert!(
            out.contains("STA CONNECTED"),
            "C3 must associate to the wifi-ap on the fast-start (browser) path"
        );
        assert!(
            fetched,
            "C3 must fetch /v1/public-stats over the modeled AP (full pipeline) on fast-start"
        );
        // The whole 165-byte body must make it out of the UART, not just the
        // first FIFO-full of it.
        assert!(
            out.contains("\"active_sessions\":4900}"),
            "the full stats body must reach the console — a truncated line means \
             the TX FIFO never drained; serial:\n{out}"
        );
        assert!(
            painted,
            "C3 must parse the stats and repaint the panel — the device is only \
             'working' once the panel shows the numbers; serial:\n{out}"
        );
    }

    fn c3_browser_fast_start_sim() -> (WasmSimulator, u32) {
        let manifest_dir = root();
        let chip_yaml =
            std::fs::read_to_string(manifest_dir.join("../../configs/chips/esp32c3.yaml"))
                .expect("read esp32c3 chip yaml");
        let system_yaml = std::fs::read_to_string(
            manifest_dir.join("../../configs/systems/esp32c3-oled-demo.yaml"),
        )
        .expect("read esp32c3-oled-demo system yaml");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip yaml");
        let manifest: SystemManifest =
            serde_yaml::from_str(&system_yaml).expect("parse system yaml");

        let irom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_rom.bin"))
            .expect("read vendored C3 IROM");
        let drom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_drom.bin"))
            .expect("read vendored C3 DROM");
        let flash = std::fs::read(manifest_dir.join("tests/fixtures/esp32c3-oled-demo-flash.bin"))
            .expect("read C3 OLED demo flash image");

        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        blobs.insert("esp32c3_irom".into(), irom);
        blobs.insert("esp32c3_drom".into(), drom);
        blobs.insert("esp32c3_flash".into(), flash);
        // Same marker blob playground injects for fast-start selection.
        blobs.insert(crate::ESP32C3_FLASH_FAST_START_BLOB.to_string(), Vec::new());

        let mut sim = WasmSimulator::new_from_config_riscv_flash_fastboot(&chip, &manifest, &blobs)
            .expect("construct C3 flash fast-start WasmSimulator (browser path)");
        let rec = sim.recommended_tick_interval();
        (sim, rec)
    }

    fn apply_browser_c3_policy(sim: &mut WasmSimulator, rec_tick: u32) {
        sim.set_peripheral_tick_interval(rec_tick);
        sim.set_idle_fast_forward_enabled(true);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod disasm_arch_tests {
    use labwired_core::decoder::arm::{decode_thumb_16, decode_thumb_32};
    use labwired_core::decoder::riscv::{decode_rv32, decode_rv32c};

    /// ADDI x1, x0, 1 — must surface as RISC-V Addi, not Thumb Unknown32.
    #[test]
    fn rv32_addi_is_not_thumb_unknown32() {
        let word: u32 = 0x0010_0093;
        let rv = format!("{:?}", decode_rv32(word));
        assert!(rv.contains("Addi"), "expected Addi, got {rv}");
        let lo = word as u16;
        let hi = (word >> 16) as u16;
        let thumb = format!("{:?}", decode_thumb_32(lo, hi));
        // The old wasm path always used Thumb: that is the bug users saw as
        // Unknown32 / Lsl / BranchCond on C3 ROM+app addresses.
        assert!(
            thumb.contains("Unknown") || !rv.eq_ignore_ascii_case(&thumb),
            "thumb decode of RV word should not look like a real RV Addi: thumb={thumb} rv={rv}"
        );
    }

    #[test]
    fn rv32c_caddi_decodes() {
        // c.addi x8, 1 — common compressed form; just ensure decode path is live.
        let hw: u16 = 0x0505;
        let s = format!("{:?}", decode_rv32c(hw));
        assert!(!s.is_empty());
        let _ = decode_thumb_16(hw);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod esp32_classic_aids_stability_tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Instant;

    fn ereader_elf_bytes() -> Option<Vec<u8>> {
        let mut candidates = Vec::new();
        if let Ok(p) = std::env::var("LABWIRED_EREADER_ELF") {
            candidates.push(PathBuf::from(p));
        }
        // cargo test -p labwired-wasm CWD is crates/wasm
        candidates.push(PathBuf::from(
            "../../../packages/playground/public/wasm/demo-labwired-ereader.elf",
        ));
        candidates.push(PathBuf::from(
            "../../packages/playground/public/wasm/demo-labwired-ereader.elf",
        ));
        candidates
            .into_iter()
            .find(|p| p.exists())
            .and_then(|p| std::fs::read(p).ok())
    }

    fn system_yaml() -> String {
        // Prefer monorepo config; fall back to minimal inline.
        let paths = [
            PathBuf::from("../../../core/configs/systems/esp32-wroom-epaper.yaml"),
            PathBuf::from("../../configs/systems/esp32-wroom-epaper.yaml"),
            PathBuf::from("../configs/systems/esp32-wroom-epaper.yaml"),
        ];
        for p in paths {
            if let Ok(s) = std::fs::read_to_string(&p) {
                return s;
            }
        }
        // Must stay byte-compatible with configs/systems/esp32-wroom-epaper.yaml
        // — the ereader ELF is a GxEPD2_290_C90c build, which emits SSD1680
        // opcodes. epaper_twin_single_source.rs fails if this copy drifts.
        r#"
name: "esp32-wroom-epaper"
chip: "esp32"
external_devices:
  - id: "epaper"
    type: "ssd1680_tricolor_290"
    connection: "spi3"
    config:
      cs_pin: "GPIO5"
      dc_pin: "GPIO17"
"#
        .to_string()
    }

    fn chip_yaml() -> String {
        let paths = [
            PathBuf::from("../../../core/configs/chips/esp32.yaml"),
            PathBuf::from("../../configs/chips/esp32.yaml"),
            PathBuf::from("../configs/chips/esp32.yaml"),
        ];
        for p in paths {
            if let Ok(s) = std::fs::read_to_string(&p) {
                return s;
            }
        }
        panic!("esp32.yaml not found for wasm aids stability test");
    }

    fn dump(sim: &WasmSimulator, label: &str) {
        let pc0 = sim.get_pc().expect("machine present");
        let sec_pc = sim
            .machine
            .as_ref()
            .and_then(|m| m.cpu_secondary.as_ref())
            .map(|c| c.get_pc())
            .unwrap_or(0);
        let parked0 = sim
            .machine
            .as_ref()
            .map(|m| m.cpu.is_parked_idle())
            .unwrap_or(false);
        let parked1 = sim
            .machine
            .as_ref()
            .and_then(|m| m.cpu_secondary.as_ref())
            .map(|c| c.is_parked_idle())
            .unwrap_or(false);
        let skipped = sim.idle_fast_forward_cycles_skipped();
        eprintln!(
            "{label}: pc0={pc0:#010x} parked0={parked0} pc1={sec_pc:#010x} parked1={parked1} skipped={skipped}"
        );
    }

    /// Exact browser entry: new_from_config + install_arduino_esp32_quirks +
    /// step_with_esp32_aids (currently dual-core → N× single).
    #[test]
    fn wasm_simulator_ereader_aids_idle_ff_does_not_fault() {
        let Some(fw) = ereader_elf_bytes() else {
            eprintln!("[skip] no ereader elf");
            return;
        };
        let mut sim =
            WasmSimulator::new_from_config(&system_yaml(), &chip_yaml(), &fw, JsValue::NULL)
                .expect("new_from_config esp32");
        sim.install_arduino_esp32_quirks(&fw)
            .expect("install quirks");
        sim.set_idle_fast_forward_enabled(true);
        let rec = sim.recommended_tick_interval();
        sim.set_peripheral_tick_interval(rec);

        let target_batches = 40u32; // 40 * 50k = 2M single-steps via aids
        let batch = 50_000u32;
        let t0 = Instant::now();
        for i in 0..target_batches {
            match sim.step_with_esp32_aids(batch) {
                Ok(()) => {}
                Err(e) => {
                    let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                    dump(&sim, &format!("FAIL batch={i} err={msg}"));
                    // Dispose safety: free/drop after error must not panic.
                    drop(sim);
                    panic!("step_with_esp32_aids fault at batch {i}: {msg}");
                }
            }
            if i % 5 == 0 {
                dump(&sim, &format!("progress batch={i}"));
            }
        }
        let wall = t0.elapsed().as_secs_f64();
        let cycles = u64::from(target_batches) * u64::from(batch);
        eprintln!(
            "OK aids: cycles={cycles} wall={wall:.3}s mips={:.3} skipped={}",
            (cycles as f64 / wall) / 1e6,
            sim.idle_fast_forward_cycles_skipped()
        );
        dump(&sim, "final");
        drop(sim);
    }

    /// Preferred path after the PR-I fix: dual-core aids should use batched
    /// AdvanceRequest::run so idle FF can engage.
    #[test]
    fn wasm_simulator_ereader_batch_run_idle_ff() {
        let Some(fw) = ereader_elf_bytes() else {
            eprintln!("[skip] no ereader elf");
            return;
        };
        let mut sim =
            WasmSimulator::new_from_config(&system_yaml(), &chip_yaml(), &fw, JsValue::NULL)
                .expect("new_from_config");
        sim.install_arduino_esp32_quirks(&fw).expect("quirks");
        sim.set_idle_fast_forward_enabled(true);
        let rec = sim.recommended_tick_interval();
        sim.set_peripheral_tick_interval(rec);

        // Drive Machine::advance(run) directly through step_batch — once aids
        // routes dual-core here, this is the browser path.
        let t0 = Instant::now();
        let mut done = 0u32;
        let target = 5_000_000u32;
        while done < target {
            let n = 200_000u32.min(target - done);
            match sim.step_batch(n) {
                Ok(e) => {
                    done = done.saturating_add(e.max(1));
                }
                Err(e) => {
                    let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                    dump(&sim, &format!("FAIL step_batch done={done} err={msg}"));
                    drop(sim);
                    panic!("step_batch fault: {msg}");
                }
            }
        }
        let wall = t0.elapsed().as_secs_f64();
        eprintln!(
            "OK step_batch: done={done} wall={wall:.3}s mips={:.3} skipped={}",
            (done as f64 / wall) / 1e6,
            sim.idle_fast_forward_cycles_skipped()
        );
        dump(&sim, "final");
        assert!(
            sim.idle_fast_forward_cycles_skipped() > 0,
            "idle FF should engage on waiti while primary parks"
        );
        drop(sim);
    }

    /// PR-I: sequential WasmSimulator sessions in one process must not inherit
    /// the prior session's fake timer / APPCPU TLS and fault at ~0x33xxxx.
    #[test]
    fn wasm_simulator_ereader_sequential_rerun_does_not_fault() {
        let Some(fw) = ereader_elf_bytes() else {
            eprintln!("[skip] no ereader elf");
            return;
        };
        for label in ["A", "B", "C"] {
            let mut sim =
                WasmSimulator::new_from_config(&system_yaml(), &chip_yaml(), &fw, JsValue::NULL)
                    .expect("new_from_config");
            sim.install_arduino_esp32_quirks(&fw).expect("quirks");
            sim.set_idle_fast_forward_enabled(true);
            let rec = sim.recommended_tick_interval();
            sim.set_peripheral_tick_interval(rec);
            match sim.step_with_esp32_aids(2_000_000) {
                Ok(()) => {
                    eprintln!(
                        "OK session {label}: skipped={}",
                        sim.idle_fast_forward_cycles_skipped()
                    );
                }
                Err(e) => {
                    let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                    dump(&sim, &format!("FAIL session={label} err={msg}"));
                    drop(sim);
                    panic!("session {label} fault: {msg}");
                }
            }
            drop(sim);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod console_tap_tests {
    //! THE TWIN MUST TAP THE CONSOLE THE BOARD'S CABLE IS ON.
    //!
    //! An ESP32-C3 has two consoles: UART0 and its own USB-Serial-JTAG. Which
    //! one carries `Serial` to the host is a BOARD fact — `deploy.usb` in the
    //! board contract:
    //!
    //!   * `native` (esp32-c3-supermini, esp32-s3-zero) — USB-C lands on the
    //!     MCU's USB-Serial-JTAG. UART0 goes to GPIO20/21 header pins with
    //!     nothing on them.
    //!   * bridge chip (classic esp32, adafruit-feather-esp32-v2) — a CP210x on
    //!     UART0 IS the USB device the host enumerates.
    //!
    //! The build side derives `ARDUINO_USB_CDC_ON_BOOT` from exactly that field.
    //! These tests are the twin's half: the run manifest declares the board's
    //! console and the Serial pane shows that console and only that console.
    //!
    //! ## The fixture
    //!
    //! One image driving BOTH consoles, so a single boot proves both directions:
    //!
    //! ```ino
    //! HWCDC UsbCdc;
    //! void setup() { Serial.begin(115200); UsbCdc.begin(); }
    //! void loop() {
    //!   Serial.println("LW_UART0_TICK");     // UART0
    //!   UsbCdc.println("LW_USBCDC_TICK");    // USB-Serial-JTAG
    //!   delay(100);
    //! }
    //! ```
    //!
    //! Built on the hosted PlatformIO toolchain for board `esp32-c3-supermini`,
    //! language `arduino`, merged at its flash offsets into
    //! `fixtures/esp32c3-usb-cdc-console-flash.bin`. It boots through the real
    //! mask ROM, so both consoles carry genuine traffic: the C3 BROM prints its
    //! banner to UART0 AND USB-Serial-JTAG, and the sketch then prints its own
    //! marker to UART0.
    //!
    //! ## What `LW_USBCDC_TICK` is NOT doing here
    //!
    //! It never appears — on either console — and that is a SEPARATE, deeper
    //! gap these tests deliberately do not paper over. `arduino-esp32`'s HWCDC
    //! (the driver a CDC-on-boot build binds to `Serial`) is entirely
    //! interrupt-driven: `HWCDC::write` only enqueues if `isCDC_Connected()`,
    //! which needs `usb_serial_jtag_is_connected()` (a SOF-frame watchdog on
    //! `INT_RAW.SOF`), and the ring buffer is only moved into the TX FIFO by the
    //! `SERIAL_IN_EMPTY` ISR. The twin's USB-Serial-JTAG model has neither —
    //! it is a polling-only byte sink (EP1_CONF permanently ready, no interrupt
    //! registers at all), which is why the mask ROM's `usb_uart_tx_one_char`
    //! busy-poll works through it and HWCDC produces nothing. Fixing the tap is
    //! necessary and not sufficient; modelling the USB host (SOF + IN_EMPTY +
    //! matrix IRQ) is the follow-up. Asserting on `LW_USBCDC_TICK` here would
    //! just fail for a reason this change is not about, so instead these tests
    //! use the traffic that IS real on both consoles.
    use super::*;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// The C3 mask ROM banner. Printed to BOTH consoles, which is why the twin
    /// cannot simply merge the two taps.
    const ROM_BANNER: &str = "ESP-ROM:esp32c3";
    /// The sketch's UART0 marker — app output, after the ROM is done.
    const UART0_MARKER: &str = "LW_UART0_TICK";

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn c3_chip() -> ChipDescriptor {
        serde_yaml::from_str(
            &std::fs::read_to_string(root().join("../../configs/chips/esp32c3.yaml"))
                .expect("read esp32c3 chip yaml"),
        )
        .expect("parse chip yaml")
    }

    /// The C3 devkit manifest, optionally declaring the board's console.
    fn c3_manifest(console: Option<&str>) -> SystemManifest {
        let mut yaml =
            std::fs::read_to_string(root().join("../../configs/systems/esp32c3-devkit.yaml"))
                .expect("read esp32c3-devkit system yaml");
        if let Some(console) = console {
            yaml.push_str(&format!("\ndebug_uart: \"{console}\"\n"));
        }
        serde_yaml::from_str(&yaml).expect("parse system yaml")
    }

    fn c3_blobs() -> HashMap<String, Vec<u8>> {
        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        blobs.insert(
            "esp32c3_irom".into(),
            std::fs::read(root().join("../core/roms/esp32c3/esp32c3_rom.bin"))
                .expect("read vendored C3 IROM"),
        );
        blobs.insert(
            "esp32c3_drom".into(),
            std::fs::read(root().join("../core/roms/esp32c3/esp32c3_drom.bin"))
                .expect("read vendored C3 DROM"),
        );
        blobs.insert(
            "esp32c3_flash".into(),
            std::fs::read(root().join("tests/fixtures/esp32c3-usb-cdc-console-flash.bin"))
                .expect("read C3 two-console flash image"),
        );
        blobs
    }

    /// What the Serial pane shows, and what the twin says was said on the
    /// console this board has no cable on. Boots the two-console fixture through
    /// the real mask ROM until the sketch's UART0 marker has appeared on one
    /// stream or the other, so neither assertion below can pass vacuously.
    fn run_two_console_fixture(console: Option<&str>) -> (String, String) {
        let chip = c3_chip();
        let manifest = c3_manifest(console);
        let mut sim = WasmSimulator::new_from_config_riscv_romboot(&chip, &manifest, &c3_blobs())
            .expect("construct C3 rom-boot WasmSimulator");

        const BATCH: u32 = 1_000_000;
        const MAX_STEPS: u64 = 400_000_000;
        let mut steps: u64 = 0;
        let shown = loop {
            let shown = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
            let unheard = String::from_utf8_lossy(&sim.unheard_console_output()).into_owned();
            if shown.contains(UART0_MARKER) || unheard.contains(UART0_MARKER) {
                break shown;
            }
            assert!(
                steps < MAX_STEPS,
                "the sketch never reached loop() within {MAX_STEPS} steps.\n\
                 --- pane ---\n{shown}\n--- unheard ---\n{unheard}"
            );
            sim.step(BATCH).expect("step");
            steps += BATCH as u64;
        };
        let unheard = String::from_utf8_lossy(&sim.unheard_console_output()).into_owned();
        (shown, unheard)
    }

    /// DIRECTION 1 — the new case. A native-USB board declares its console, and
    /// the pane shows what the USB-C cable carries: the ROM's USB-Serial-JTAG
    /// traffic, and NOT the UART0 stream, which on a SuperMini goes to bare
    /// header pins. Before this change the tap could only be UART0 unless a lab
    /// hand-authored `debug_uart`, and nothing derived it from the board.
    #[test]
    #[ignore = "boots the real C3 mask ROM (~150M steps); run with --release --ignored"]
    fn native_usb_board_shows_the_usb_console() {
        let (shown, unheard) = run_two_console_fixture(Some("usb_serial_jtag"));

        assert!(
            shown.contains(ROM_BANNER),
            "the USB-Serial-JTAG console carried no traffic at all:\n{shown}"
        );
        // Non-vacuous: UART0 demonstrably HAD app output at this point — it is
        // sitting in the unheard stream — and it stayed out of the USB pane.
        assert!(
            unheard.contains(UART0_MARKER),
            "UART0 never printed, so 'UART0 stays out of the pane' proves nothing:\n{unheard}"
        );
        assert!(
            !shown.contains(UART0_MARKER),
            "UART0 app output leaked into a native-USB board's pane — GPIO20/21 \
             have nothing on them on a SuperMini:\n{shown}"
        );
    }

    /// DIRECTION 2 — no regression. An undeclared manifest (every lab shipped so
    /// far) still shows UART0, and still shows the ROM banner exactly once.
    /// That count is the load-bearing part: the ROM prints the same banner to
    /// both consoles, so a twin that merged the two taps to make the new case
    /// "work" would double every ROM character here.
    #[test]
    #[ignore = "boots the real C3 mask ROM (~150M steps); run with --release --ignored"]
    fn bridge_console_board_still_shows_uart0() {
        let (shown, unheard) = run_two_console_fixture(None);

        assert!(
            shown.contains(UART0_MARKER),
            "UART0 console regressed out of the Serial pane:\n{shown}"
        );
        assert_eq!(
            shown.matches(ROM_BANNER).count(),
            1,
            "ROM banner is not printed exactly once — the two taps got merged:\n{shown}"
        );
        // The USB console said nothing UART0 did not also say, so there is
        // nothing to report and the pane is the whole story.
        assert!(
            unheard.is_empty(),
            "nothing should be unheard on a UART0-console board here:\n{unheard}"
        );
    }

    /// The failure this change exists to stop being SILENT. With the board's
    /// cable on USB, the sketch's UART0 output reaches no connector — a real
    /// SuperMini shows nothing, and so does the twin. But the twin can SAY so,
    /// which is the difference between "empty pane" and "empty pane for a
    /// reason". The ROM banner both consoles received is not counted.
    #[test]
    #[ignore = "boots the real C3 mask ROM (~150M steps); run with --release --ignored"]
    fn output_on_the_untapped_console_is_reported_not_lost() {
        let (shown, unheard) = run_two_console_fixture(Some("usb_serial_jtag"));

        assert!(
            unheard.contains(UART0_MARKER),
            "firmware printed to a console with no connector and the twin could \
             not say so.\n--- pane ---\n{shown}\n--- unheard ---\n{unheard}"
        );
        assert!(
            !unheard.contains(ROM_BANNER),
            "the banner BOTH consoles received was counted as unheard output, \
             which would raise the alarm on every single run:\n{unheard}"
        );
    }
}

#[cfg(test)]
mod cpu_inspector_boundary_tests {
    use super::*;

    /// The exact ELF the motor-parity test already boots, loaded through the
    /// legacy Cortex-M constructor: flash at 0x0800_0000, RAM at 0x2000_0000.
    fn sim() -> WasmSimulator {
        let fw = include_bytes!("../tests/fixtures/firmware-l476-bldc-six-step.elf");
        WasmSimulator::new(fw).expect("fixture ELF loads on the legacy Cortex-M bus")
    }

    /// Converting the CPU-inspector accessors to `Result` must not change what a
    /// working board reports — only what an unanswerable read reports.
    ///
    /// Cross-checked against `peek`, which reaches the same bytes by a different
    /// route (`Machine::peek`, side-effect free) than `read_memory` (the real
    /// bus read path). Two independent paths agreeing on the flash image is a
    /// byte-identity check, not a restatement of the implementation.
    #[test]
    fn a_working_board_still_reads_the_same_bytes() {
        let sim = sim();

        let via_bus = sim
            .read_memory(0x0800_0000, 64)
            .expect("mapped flash reads");
        let via_peek = sim.peek(0x0800_0000, 64).expect("mapped flash peeks");
        assert_eq!(
            via_bus,
            via_peek.to_vec(),
            "read_memory and peek disagree on mapped flash"
        );

        // The reset vector: a real value, and specifically not the zeros the old
        // `unwrap_or(0)` would have been indistinguishable from.
        assert_ne!(&via_bus[..8], &[0u8; 8], "flash read came back all zeros");
        assert_eq!(
            sim.read_memory(0x2000_0000, 16)
                .expect("mapped RAM reads")
                .len(),
            16
        );
    }

    /// The register accessors used to `.unwrap()` on `self.machine`, which
    /// panics out through the wasm frame and permanently poisons the
    /// wasm-bindgen borrow guard. On a live machine they must simply answer.
    #[test]
    fn the_register_path_answers_on_a_live_machine() {
        let sim = sim();
        let pc = sim.get_pc().expect("live machine has a PC");
        assert_ne!(pc, 0, "PC read back as 0");
        sim.get_register(0).expect("live machine has r0");
        // `get_register_names` is deliberately not called: it serializes through
        // `serde_wasm_bindgen`, which is `unreachable!()` off wasm32. That was
        // already true of the `.unwrap()` it used to do.
    }

    // The failure half of this contract — that an unmapped `read_memory` now
    // returns `Err` instead of a page of fabricated zeros — cannot be asserted
    // here. Building the error value calls `JsValue::from_str`, and `JsValue`
    // is `unreachable!()` off the wasm32 target, so a native test panics inside
    // wasm-bindgen before it can observe the `Err`. It is covered by the
    // signature itself (`Result<Vec<u8>, JsValue>` has no way to express the
    // old zero-fill) and by the `error_boundary_ratchet` scan.
}
