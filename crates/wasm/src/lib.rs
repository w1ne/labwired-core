use labwired_config::{
    Arch, BoardIoBinding, BoardIoKind, BoardIoSignal, ChipDescriptor, SystemManifest,
};
use labwired_core::bus::SystemBus;

// #124 Phase 4: browser-side JIT prototype. Runs the dominant
// `0x400829cc` hot block through `js_sys::WebAssembly` instead of the
// interpreter when `jit_enabled()` has been toggled on from JS.
mod inputs;
mod inspect;
mod install;
mod jit_browser;
mod traces;
// CortexM and XtensaLx7 are used via Box<dyn Cpu>; the concrete types are
// only constructed inside the configure_* fns and immediately boxed.
use labwired_core::decoder::arm::{decode_thumb_16, decode_thumb_32};
use labwired_core::memory::LinearMemory;
use labwired_core::peripherals::adc::Adc;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::Bus;
use labwired_core::{Cpu, Machine};
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

#[wasm_bindgen]
pub struct WasmSimulator {
    machine: Option<Machine<Box<dyn Cpu>>>,
    board_io: Vec<BoardIoBinding>,
    uart_sink: Arc<Mutex<Vec<u8>>>,
    uart_rx_bufs: Vec<Arc<Mutex<VecDeque<u8>>>>,
    #[allow(dead_code)]
    arch: Arch,
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

#[wasm_bindgen]
impl WasmSimulator {
    /// Legacy constructor: hardcoded STM32F107 Cortex-M3 with 128KB flash + 20KB RAM.
    /// Kept for backward compatibility with the existing landing page sandbox.
    #[wasm_bindgen(constructor)]
    pub fn new(firmware: &[u8]) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::new();
        bus.flash = LinearMemory::new(128 * 1024, 0x0800_0000);
        bus.ram = LinearMemory::new(20 * 1024, 0x2000_0000);
        bus.refresh_peripheral_index();

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
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
            uart_rx_bufs,
            arch: Arch::Arm,
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
    ) -> Result<WasmSimulator, JsValue> {
        let manifest: SystemManifest = serde_yaml::from_str(system_yaml)
            .map_err(|e| JsValue::from_str(&format!("System YAML error: {}", e)))?;
        let chip: ChipDescriptor = serde_yaml::from_str(chip_yaml)
            .map_err(|e| JsValue::from_str(&format!("Chip YAML error: {}", e)))?;

        match chip.arch {
            Arch::Arm | Arch::Unknown => Self::new_from_config_arm(&chip, &manifest, firmware),
            Arch::RiscV => Self::new_from_config_riscv(&chip, &manifest, firmware),
            Arch::Xtensa => Self::new_from_config_xtensa_esp32(&manifest, firmware),
        }
    }

    fn new_from_config_arm(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        firmware: &[u8],
    ) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        if let Some(debug_uart) = manifest.debug_uart.as_deref() {
            if !bus.attach_uart_tx_sink_named(debug_uart, uart_sink.clone(), false) {
                bus.attach_uart_tx_sink(uart_sink.clone(), false);
            }
        } else {
            bus.attach_uart_tx_sink(uart_sink.clone(), false);
        }
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
            uart_rx_bufs,
            arch: Arch::Arm,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// RISC-V (esp32c3) bus setup. Mirrors `new_from_config_arm` but builds a
    /// RISC-V core via `configure_riscv` and seeds the stack pointer at the top
    /// of DRAM — fast-boot skips the ROM/2nd-stage bootloader that would
    /// normally set SP, so the app's first prologue store would otherwise fault.
    fn new_from_config_riscv(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        firmware: &[u8],
    ) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        if let Some(debug_uart) = manifest.debug_uart.as_deref() {
            if !bus.attach_uart_tx_sink_named(debug_uart, uart_sink.clone(), false) {
                bus.attach_uart_tx_sink(uart_sink.clone(), false);
            }
        } else {
            bus.attach_uart_tx_sink(uart_sink.clone(), false);
        }
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let mut machine = Machine::new(boxed, bus);

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        machine
            .load_firmware(&program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;

        let sp_top =
            (chip.ram.base + labwired_config::parse_size(&chip.ram.size).unwrap_or(0)) as u32;
        machine.cpu.set_sp(sp_top & !0xF);

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            uart_rx_bufs,
            arch: Arch::RiscV,
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
        let mut bus = SystemBus::new();
        let cpu = configure_xtensa_esp32(&mut bus);

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(uart_sink.clone(), false);
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
            uart_rx_bufs,
            arch: Arch::Xtensa,
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

        let snapshot = machine.bus.peripherals[idx].dev.snapshot();

        let register = match binding.kind {
            BoardIoKind::Led | BoardIoKind::PwmOutput => "odr",
            BoardIoKind::Button => "idr",
            // Analog/bus kinds are not boolean and are exposed through typed state accessors.
            BoardIoKind::AdcInput
            | BoardIoKind::I2cDevice
            | BoardIoKind::SpiDevice
            | BoardIoKind::UartDevice => {
                return false;
            }
        };
        let reg_val = snapshot[register].as_u64().unwrap_or(0) as u32;
        let pin_high = (reg_val >> binding.pin) & 1 == 1;

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
                .step()
                .map_err(|e| JsValue::from_str(&format!("Step Error: {}", e)))?;
        }
        Ok(())
    }

    #[wasm_bindgen]
    pub fn step_single(&mut self) -> Result<(), JsValue> {
        self.machine()
            .step()
            .map_err(|e| JsValue::from_str(&format!("Step Error: {}", e)))
    }

    /// Connect this chip's UART (`uart_id`, e.g. "uart2") to a shared in-module
    /// cross-link, so it exchanges bytes with the other chip on the same
    /// `link_id`. The two chips of a point-to-point IO-Link use opposite
    /// `side`s (0 and 1). Bytes flow through a process-static medium with no
    /// per-byte host round-trip, so both chips can keep stepping in batches.
    #[wasm_bindgen]
    pub fn attach_uart_wire(
        &mut self,
        uart_id: &str,
        link_id: u32,
        side: u8,
    ) -> Result<(), JsValue> {
        let endpoint = Box::new(
            labwired_core::network::virtual_uart_wire::VirtualWireEndpoint::new(link_id, side),
        );
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

    #[wasm_bindgen]
    pub fn get_pc(&self) -> u32 {
        self.machine.as_ref().unwrap().cpu.get_pc()
    }

    #[wasm_bindgen]
    pub fn get_register(&self, id: u8) -> u32 {
        self.machine.as_ref().unwrap().cpu.get_register(id)
    }

    #[wasm_bindgen]
    pub fn get_register_names(&self) -> JsValue {
        let names = self.machine.as_ref().unwrap().cpu.get_register_names();
        serde_wasm_bindgen::to_value(&names).unwrap()
    }

    #[wasm_bindgen]
    pub fn read_memory(&self, addr: u32, len: u32) -> Vec<u8> {
        let machine = self.machine.as_ref().unwrap();
        (0..len)
            .map(|i| machine.bus.read_u8(addr as u64 + i as u64).unwrap_or(0))
            .collect()
    }

    #[wasm_bindgen]
    pub fn get_disassembly(&self) -> String {
        let machine = self.machine.as_ref().unwrap();
        let pc = machine.cpu.get_pc() & !1;
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

    /// Execute up to max_cycles steps, returning the number actually executed.
    #[wasm_bindgen]
    pub fn step_batch(&mut self, max_cycles: u32) -> Result<u32, JsValue> {
        let machine = self.machine();
        for i in 0..max_cycles {
            if let Err(e) = machine.step() {
                return if i > 0 {
                    Ok(i)
                } else {
                    Err(JsValue::from_str(&format!("Step Error: {}", e)))
                };
            }
        }
        Ok(max_cycles)
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
    pub fn set_jit_enabled(&mut self, enabled: bool) {
        self.jit_browser_enabled = enabled;
        if !enabled {
            // Cleanly drop the cached module + closures so the next
            // enable rebuilds from scratch.
            self.jit_browser_cache = None;
        }
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
    #[wasm_bindgen]
    pub fn step_with_esp32_aids(&mut self, cycles: u32) -> Result<(), JsValue> {
        // Real dual-core: a genuine APP_CPU is attached, so the handshake
        // keep-alive and the FROM_CPU IPI bridge below are unnecessary — the
        // firmware drives the rendezvous itself and Machine::step delivers the
        // cross-core IPI via the DPORT. Just step both cores.
        if self
            .machine
            .as_ref()
            .is_some_and(|m| m.cpu_secondary.is_some())
        {
            return self.step(cycles);
        }
        if self.esp32_ipi.is_none() {
            return self.step(cycles);
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

/// Clear every shared UART cross-link. The playground calls this when (re)loading
/// a multi-chip lab so a previous station's link buffers don't leak bytes into
/// the new one.
#[wasm_bindgen]
pub fn clear_uart_wires() {
    labwired_core::network::virtual_uart_wire::clear_virtual_uart_wires();
}

// WasmGdbEventLoop removed — see `gdb_process_packet` above for the rationale.
// Restoring this requires `LabwiredTarget` to be implemented for an arch-erased
// CPU type, which is the follow-up tracked alongside Phase 1.
