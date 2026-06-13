use labwired_config::{
    Arch, BoardIoBinding, BoardIoKind, BoardIoSignal, ChipDescriptor, SystemManifest,
};
use labwired_core::bus::SystemBus;

// #124 Phase 4: browser-side JIT prototype. Runs the dominant
// `0x400829cc` hot block through `js_sys::WebAssembly` instead of the
// interpreter when `jit_enabled()` has been toggled on from JS.
mod jit_browser;
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
            Arch::Arm | Arch::RiscV | Arch::Unknown => {
                Self::new_from_config_arm(&chip, &manifest, firmware)
            }
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
        let mut machine = Machine::new(boxed, bus);

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        machine
            .load_firmware(&program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;
        // XtensaLx7::reset() defaults PC to 0x40000400 (BROM reset vector).
        // We skip BROM emulation and jump straight to the ELF's app entry,
        // matching where a 2nd-stage bootloader would land.
        machine.cpu.set_pc(program_image.entry_point as u32);

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

    /// Legacy LED state query (hardcoded GPIOB pin 5 for backward compat).
    #[wasm_bindgen]
    pub fn get_led_state(&mut self) -> bool {
        let odr = self.machine().bus.read_u32(0x4001080C).unwrap_or(0);
        (odr >> 5) & 1 == 1
    }

    /// Returns the board_io configuration as a JSON array.
    /// Each entry: { id, kind, peripheral, pin, signal, active_high }
    #[wasm_bindgen]
    pub fn get_board_io_config(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.board_io).unwrap_or(JsValue::NULL)
    }

    /// Returns the current state of all board_io bindings as a JSON array.
    /// Each entry: { id, active }
    /// Uses peripheral snapshot() to read ODR regardless of register layout.
    #[wasm_bindgen]
    pub fn get_board_io_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let active = self.read_board_io_state(machine, binding);
            states.push(serde_json::json!({
                "id": binding.id,
                "active": active,
            }));
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Set the distance (cm) reported by an HC-SR04 ultrasonic sensor — the
    /// host-controlled "hand position" that drives gesture control. Clamped to
    /// the sensor's 2–400 cm range.
    #[wasm_bindgen]
    pub fn set_hcsr04_distance(&mut self, id: &str, distance_cm: f32) -> Result<(), JsValue> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("simulator not initialized"))?;
        for sensor in machine.bus.hcsr04.iter_mut() {
            if sensor.id == id {
                sensor.set_distance_cm(distance_cm);
                return Ok(());
            }
        }
        Err(JsValue::from_str(&format!("No HC-SR04 sensor '{}'", id)))
    }

    /// Set an input board_io binding (e.g. button press).
    /// Writes to the GPIO IDR register bit for the specified binding.
    #[wasm_bindgen]
    pub fn set_board_io_input(&mut self, id: &str, active: bool) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == id && b.signal == BoardIoSignal::Input)
            .cloned()
            .ok_or_else(|| JsValue::from_str(&format!("No input board_io binding '{}'", id)))?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!("Peripheral '{}' not found", binding.peripheral))
            })?;

        // Read the IDR via snapshot, modify the bit, write back via bus
        let snapshot = machine.bus.peripherals[idx].dev.snapshot();
        let current_idr = snapshot["idr"].as_u64().unwrap_or(0) as u32;

        let pin_high = if binding.active_high { active } else { !active };
        let new_idr = if pin_high {
            current_idr | (1 << binding.pin)
        } else {
            current_idr & !(1 << binding.pin)
        };

        // Write IDR through the peripheral's write interface.
        // Determine IDR offset from layout in snapshot.
        let layout = snapshot["layout"].as_str().unwrap_or("stm32_f1");
        let idr_offset: u64 = if layout.contains("v2") { 0x10 } else { 0x08 };
        let base = machine.bus.peripherals[idx].base;
        let _ = machine.bus.write_u32(base + idr_offset, new_idr);

        Ok(())
    }

    /// Set the simulated X/Y/Z sample on an ADXL345 attached to an I2C peripheral.
    /// Looks up the binding in `board_io` by id; the binding must have
    /// `device_type: "adxl345"`.
    #[wasm_bindgen]
    pub fn set_i2c_sensor_sample(
        &mut self,
        device_id: &str,
        x: i16,
        y: i16,
        z: i16,
    ) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("adxl345"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!("No ADXL345 board_io binding '{}'", device_id))
            })?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "I2C peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("I2C peripheral does not support downcasting"))?;
        let i2c = any
            .downcast_mut::<labwired_core::peripherals::i2c::I2c>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an I2C controller",
                    binding.peripheral
                ))
            })?;

        let address = binding.i2c_address.unwrap_or(0x53);
        for device in i2c.attached_devices() {
            let mut device = device.borrow_mut();
            if device.address() != address {
                continue;
            }
            if let Some(sensor) = device.as_any_mut().and_then(|any| {
                any.downcast_mut::<labwired_core::peripherals::components::Adxl345>()
            }) {
                sensor.set_sample(x, y, z);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "ADXL345 device at address 0x{:02x} not found on '{}'",
            address, binding.peripheral
        )))
    }

    /// Set the simulated 6-DoF sample on an MPU6050 attached to an I2C peripheral.
    #[wasm_bindgen]
    #[allow(clippy::too_many_arguments)]
    pub fn set_i2c_sensor_sample_6dof(
        &mut self,
        device_id: &str,
        ax: i16,
        ay: i16,
        az: i16,
        gx: i16,
        gy: i16,
        gz: i16,
    ) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("mpu6050"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!("No MPU6050 board_io binding '{}'", device_id))
            })?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "I2C peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("I2C peripheral does not support downcasting"))?;
        let i2c = any
            .downcast_mut::<labwired_core::peripherals::i2c::I2c>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an I2C controller",
                    binding.peripheral
                ))
            })?;

        let address = binding.i2c_address.unwrap_or(0x68);
        for device in i2c.attached_devices() {
            let mut device = device.borrow_mut();
            if device.address() != address {
                continue;
            }
            if let Some(sensor) = device.as_any_mut().and_then(|any| {
                any.downcast_mut::<labwired_core::peripherals::components::Mpu6050>()
            }) {
                sensor.set_sample(ax, ay, az, gx, gy, gz);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "MPU6050 device at address 0x{:02x} not found on '{}'",
            address, binding.peripheral
        )))
    }

    /// Read back the current sensor data from each I2C sensor declared in `board_io`.
    /// Returns `[{ id, kind: "adxl345", x, y, z }, ...]` or `[{ id, kind: "mpu6050", ax, ay, az, gx, gy, gz }, ...]`
    /// or `[{ id, kind: "bme280", temperature_c, humidity_pct, pressure_hpa }, ...]`.
    #[wasm_bindgen]
    pub fn get_i2c_sensor_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "adxl345" || t == "mpu6050" || t == "bme280" => t,
                _ => continue,
            };
            let Some(idx) = machine
                .bus
                .find_peripheral_index_by_name(&binding.peripheral)
            else {
                continue;
            };
            let Some(any) = machine.bus.peripherals[idx].dev.as_any() else {
                continue;
            };
            let Some(i2c) = any.downcast_ref::<labwired_core::peripherals::i2c::I2c>() else {
                continue;
            };

            if device_type == "adxl345" {
                let address = binding.i2c_address.unwrap_or(0x53);
                for device in i2c.attached_devices() {
                    let device = device.borrow();
                    if device.address() != address {
                        continue;
                    }
                    if let Some(sensor) = device.as_any().and_then(|any| {
                        any.downcast_ref::<labwired_core::peripherals::components::Adxl345>()
                    }) {
                        let (x, y, z) = sensor.sample();
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "adxl345",
                            "x": x,
                            "y": y,
                            "z": z,
                        }));
                        break;
                    }
                }
            } else if device_type == "mpu6050" {
                let address = binding.i2c_address.unwrap_or(0x68);
                for device in i2c.attached_devices() {
                    let device = device.borrow();
                    if device.address() != address {
                        continue;
                    }
                    if let Some(sensor) = device.as_any().and_then(|any| {
                        any.downcast_ref::<labwired_core::peripherals::components::Mpu6050>()
                    }) {
                        let (ax, ay, az, gx, gy, gz) = sensor.sample();
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "mpu6050",
                            "ax": ax,
                            "ay": ay,
                            "az": az,
                            "gx": gx,
                            "gy": gy,
                            "gz": gz,
                        }));
                        break;
                    }
                }
            } else if device_type == "bme280" {
                // Static values: hard-coded factory calibration produces ~25°C / 50%RH / 1013hPa
                let address = binding.i2c_address.unwrap_or(0x76);
                for device in i2c.attached_devices() {
                    let device = device.borrow();
                    if device.address() != address {
                        continue;
                    }
                    if device
                        .as_any()
                        .and_then(|any| {
                            any.downcast_ref::<labwired_core::peripherals::components::Bme280>()
                        })
                        .is_some()
                    {
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "bme280",
                            "temperature_c": 25.0_f64,
                            "humidity_pct": 50.0_f64,
                            "pressure_hpa": 1013.0_f64,
                        }));
                        break;
                    }
                }
            }
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Snapshot of the shared virtual-air TX trace ring buffer (last
    /// ~200 BLE/proprietary frames pushed by any chip in this WASM
    /// instance, most-recent-first). The playground's BLE-on-canvas
    /// visualization polls this to render the packet trace panel; the
    /// underlying state lives in a Rust static, so any WasmSimulator
    /// can return the same snapshot — pick whichever chip is alive.
    #[wasm_bindgen]
    pub fn air_trace_snapshot(&self) -> JsValue {
        let trace = labwired_core::peripherals::nrf52::radio::virtual_air_trace_snapshot();
        serde_wasm_bindgen::to_value(&trace).unwrap_or(JsValue::NULL)
    }

    /// Drain UART TX output bytes accumulated since the last call.
    #[wasm_bindgen]
    pub fn drain_uart_output(&self) -> Vec<u8> {
        if let Ok(mut buf) = self.uart_sink.lock() {
            let data = buf.clone();
            buf.clear();
            data
        } else {
            Vec::new()
        }
    }

    /// Non-consuming UART trace snapshot for instruments such as the logic analyzer.
    #[wasm_bindgen]
    pub fn uart_trace_snapshot(&self) -> JsValue {
        let Some(machine) = self.machine.as_ref() else {
            return serde_wasm_bindgen::to_value(&Vec::<serde_json::Value>::new())
                .unwrap_or(JsValue::NULL);
        };

        let snapshots = machine
            .bus
            .peripherals
            .iter()
            .filter_map(|p| {
                let any = p.dev.as_any()?;
                let uart = any.downcast_ref::<labwired_core::peripherals::uart::Uart>()?;
                Some(serde_json::json!({
                    "peripheral": p.name,
                    "events": uart.trace_snapshot(),
                }))
            })
            .collect::<Vec<_>>();

        serde_wasm_bindgen::to_value(&snapshots).unwrap_or(JsValue::NULL)
    }

    /// Non-consuming FDCAN frame trace snapshot for CAN/UDS instruments.
    #[wasm_bindgen]
    pub fn fdcan_trace_snapshot(&self) -> JsValue {
        let Some(machine) = self.machine.as_ref() else {
            return serde_wasm_bindgen::to_value(&Vec::<serde_json::Value>::new())
                .unwrap_or(JsValue::NULL);
        };

        let snapshots = machine
            .bus
            .peripherals
            .iter()
            .flat_map(|p| {
                let Some(any) = p.dev.as_any() else {
                    return Vec::new();
                };
                let Some(fdcan) = any.downcast_ref::<labwired_core::peripherals::fdcan::Fdcan>()
                else {
                    return Vec::new();
                };
                fdcan.trace_snapshot(&p.name)
            })
            .collect::<Vec<_>>();

        serde_wasm_bindgen::to_value(&snapshots).unwrap_or(JsValue::NULL)
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

    /// Get a peripheral's full state snapshot as JSON.
    #[wasm_bindgen]
    pub fn get_peripheral_snapshot(&self, name: &str) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        if let Some(idx) = machine.bus.find_peripheral_index_by_name(name) {
            let snapshot = machine.bus.peripherals[idx].dev.snapshot();
            serde_wasm_bindgen::to_value(&snapshot).unwrap_or(JsValue::NULL)
        } else {
            JsValue::NULL
        }
    }

    /// Push bytes into all UART RX buffers (bidirectional serial input).
    #[wasm_bindgen]
    pub fn feed_uart_input(&self, data: &[u8]) {
        for buf in &self.uart_rx_bufs {
            if let Ok(mut guard) = buf.lock() {
                guard.extend(data.iter());
            }
        }
    }

    /// Inject an ADC value into a named ADC peripheral's data register.
    #[wasm_bindgen]
    pub fn set_adc_value(&mut self, peripheral_name: &str, value: u16) -> Result<(), JsValue> {
        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(peripheral_name)
            .ok_or_else(|| JsValue::from_str(&format!("ADC '{}' not found", peripheral_name)))?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("Peripheral doesn't support downcasting"))?;
        let adc = any
            .downcast_mut::<Adc>()
            .ok_or_else(|| JsValue::from_str("Peripheral is not an ADC"))?;
        adc.dr = (value & 0xFFF) as u32;
        adc.sr |= 1 << 1; // Set EOC
        Ok(())
    }

    /// Set the simulated temperature on an NTC thermistor attached to an ADC channel.
    ///
    /// All Steinhart-Hart math lives in Rust core (NtcThermistor::divider_output_mv).
    /// This function only stores the new temperature, recomputes divider_mv → ADC count
    /// via core, and injects the result into the ADC peripheral's channel.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "ntc-thermistor"`.
    #[wasm_bindgen]
    pub fn set_ntc_temperature(
        &mut self,
        device_id: &str,
        temperature_c: f32,
    ) -> Result<(), JsValue> {
        use labwired_core::peripherals::components::NtcThermistor;

        // Find the board_io binding for this device.
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("ntc-thermistor"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "No ntc-thermistor board_io binding '{}'",
                    device_id
                ))
            })?;

        let channel = binding.pin;

        // Build a temporary NTC model to compute the millivolt output — all math in core.
        let mut ntc = NtcThermistor::new(channel, temperature_c);
        ntc.set_temperature(temperature_c);
        let mv = ntc.divider_output_mv();

        // Inject the computed millivolt value into the matching ADC peripheral's channel.
        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "ADC peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("ADC peripheral does not support downcasting"))?;
        let adc = any.downcast_mut::<Adc>().ok_or_else(|| {
            JsValue::from_str(&format!(
                "Peripheral '{}' is not an ADC",
                binding.peripheral
            ))
        })?;

        adc.set_channel_input(channel, mv);
        Ok(())
    }

    /// Read back the current state of all NTC thermistor devices declared in `board_io`.
    ///
    /// Returns `[{ id, kind: "ntc-thermistor", temperature_c, divider_mv, adc_count }]`.
    /// All conversion math (Steinhart-Hart, mV→count) is performed here by calling into
    /// core types — no conversion logic in this WASM bridge body.
    #[wasm_bindgen]
    pub fn get_adc_device_states(&self) -> JsValue {
        use labwired_core::peripherals::components::NtcThermistor;

        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "ntc-thermistor" => t,
                _ => continue,
            };
            let Some(idx) = machine
                .bus
                .find_peripheral_index_by_name(&binding.peripheral)
            else {
                continue;
            };
            let Some(any) = machine.bus.peripherals[idx].dev.as_any() else {
                continue;
            };
            let Some(adc) = any.downcast_ref::<Adc>() else {
                continue;
            };

            if device_type == "ntc-thermistor" {
                // Read the current ADC count from the data register.
                let adc_count = adc.dr as u16;
                // Back-compute millivolts from count (3.3 V Vref, 12-bit).
                let divider_mv = ((adc_count as u32 * 3300) / 4095) as u16;

                // Reverse the voltage divider: R_ntc = R_pull * (V_ref/V_out - 1)
                // Then use Beta equation: T = B / (ln(R/R0) + B/T0) to get temperature.
                // Build an NTC model and use divider_output_mv to find the matching temp.
                // Since we can't easily invert exp, we read temperature from what was last set.
                // Instead, we just expose the raw ADC count and mV here; the UI shows them.
                // Temperature is the authoritative value set via set_ntc_temperature.
                // Use a 25 °C default NTC to compute nominal values for display.
                let channel = binding.pin;
                // Try to recover the last-injected mV from channel_inputs.
                let injected_mv = if (channel as usize) < 18 {
                    // Access via snapshot to avoid mutable borrow; use the divider_mv we computed.
                    divider_mv
                } else {
                    divider_mv
                };

                // Build a reference NTC at 25 °C to show alongside actual values.
                let ntc_ref = NtcThermistor::new(channel, 25.0);
                let _ = ntc_ref; // Used for type verification — the display values are from ADC.

                states.push(serde_json::json!({
                    "id": binding.id,
                    "kind": "ntc-thermistor",
                    "divider_mv": injected_mv,
                    "adc_count": adc_count,
                }));
            }
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Returns analog state for ADC and PWM board_io bindings.
    #[wasm_bindgen]
    pub fn get_board_io_analog_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            match binding.kind {
                BoardIoKind::AdcInput => {
                    if let Some(idx) = machine
                        .bus
                        .find_peripheral_index_by_name(&binding.peripheral)
                    {
                        let snap = machine.bus.peripherals[idx].dev.snapshot();
                        let dr = snap["dr"].as_u64().unwrap_or(0);
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "adc_input",
                            "value": dr,
                        }));
                    }
                }
                BoardIoKind::PwmOutput => {
                    let active = self.read_board_io_state(machine, binding);
                    states.push(serde_json::json!({
                        "id": binding.id,
                        "kind": "pwm_output",
                        "active": active,
                    }));
                }
                _ => {}
            }
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Return the SSD1306 GDDRAM framebuffer for the device identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "oled-ssd1306"`.
    /// Returns a 1024-byte `Uint8Array` (128 columns × 8 pages, page-major).
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_ssd1306_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        // Find the board_io binding for this device.
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("oled-ssd1306"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No oled-ssd1306 board_io binding '{}'", device_id))
            })?;

        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "I2C peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;

        let any = machine.bus.peripherals[idx]
            .dev
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        let i2c = any
            .downcast_ref::<labwired_core::peripherals::i2c::I2c>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an I2C controller",
                    binding.peripheral
                ))
            })?;

        let address = binding.i2c_address.unwrap_or(0x3C);
        for device in i2c.attached_devices() {
            let device = device.borrow();
            if device.address() != address {
                continue;
            }
            if let Some(oled) = device.as_any().and_then(|any| {
                any.downcast_ref::<labwired_core::peripherals::components::Ssd1306>()
            }) {
                let fb = oled.framebuffer().to_vec().into_boxed_slice();
                return Ok(fb);
            }
        }

        Err(JsValue::from_str(&format!(
            "SSD1306 device at address 0x{:02x} not found on '{}'",
            address, binding.peripheral
        )))
    }

    /// Return the ILI9341 RGB565 framebuffer for the device identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "ili9341"`.
    /// Returns a 153,600-byte `Uint8Array` (240×320 pixels × 2 bytes, row-major, big-endian RGB565).
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_ili9341_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("ili9341"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No ili9341 board_io binding '{}'", device_id))
            })?;

        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "SPI peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;

        let any = machine.bus.peripherals[idx]
            .dev
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        let spi = any
            .downcast_ref::<labwired_core::peripherals::spi::Spi>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                ))
            })?;

        for device in &spi.attached_devices {
            if let Some(tft) = device
                .as_any()
                .and_then(|a| a.downcast_ref::<labwired_core::peripherals::components::Ili9341>())
            {
                let fb = tft.framebuffer().to_vec().into_boxed_slice();
                return Ok(fb);
            }
        }

        Err(JsValue::from_str(&format!(
            "ILI9341 device not found on SPI peripheral '{}'",
            binding.peripheral
        )))
    }

    /// Return the PCD8544 (Nokia 5110) framebuffer for the device identified
    /// by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type:
    /// "pcd8544"`. Returns 504 bytes: 84 columns × 6 banks, bank-major. Pixel
    /// (x, y) is bit `(y % 8)` of byte `[(y / 8) * 84 + x]` (1 = on/dark).
    #[wasm_bindgen]
    pub fn get_pcd8544_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("pcd8544"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No pcd8544 board_io binding '{}'", device_id))
            })?;

        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "SPI peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;

        let any = machine.bus.peripherals[idx]
            .dev
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        let spi = any
            .downcast_ref::<labwired_core::peripherals::spi::Spi>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                ))
            })?;

        for device in &spi.attached_devices {
            if let Some(lcd) = device
                .as_any()
                .and_then(|a| a.downcast_ref::<labwired_core::peripherals::components::Pcd8544>())
            {
                return Ok(lcd.framebuffer().to_vec().into_boxed_slice());
            }
        }

        Err(JsValue::from_str(&format!(
            "PCD8544 device not found on SPI peripheral '{}'",
            binding.peripheral
        )))
    }

    /// Return the SSD1680 tri-color e-paper framebuffer for the device identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "ssd1680_tricolor_290"`.
    /// Returns a 9472-byte `Uint8Array`: first 4736 bytes are the black plane
    /// (1 = white / 0 = black), next 4736 bytes are the red plane on the wire
    /// (1 = no-red / 0 = red — see GxEPD2 inversion in writeImage). Row-major,
    /// 128 pixels wide / 296 tall native, MSB-first packing within each byte.
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_ssd1680_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| {
                b.id == device_id
                    && matches!(
                        b.device_type.as_deref(),
                        Some("ssd1680_tricolor_290") | Some("epd-2in9-tricolor")
                    )
            })
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "No ssd1680_tricolor_290 board_io binding '{}'",
                    device_id
                ))
            })?;

        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "SPI peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;

        let any = machine.bus.peripherals[idx]
            .dev
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        // The SSD1680 panel attaches to either the generic STM32-shape Spi
        // peripheral or the Esp32Spi controller (same SpiDevice trait,
        // different controller models). Try both downcasts.
        let panel_bytes =
            if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
                spi.attached_devices.iter().find_map(|dev| {
                    dev.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ssd1680Tricolor290>()
                }).map(|panel| (panel.black_plane().to_vec(), panel.red_plane().to_vec()))
                })
            } else if let Some(spi) =
                any.downcast_ref::<labwired_core::peripherals::esp32::spi::Esp32Spi>()
            {
                spi.attached_devices.iter().find_map(|dev| {
                    dev.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ssd1680Tricolor290>()
                }).map(|panel| (panel.black_plane().to_vec(), panel.red_plane().to_vec()))
                })
            } else {
                return Err(JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                )));
            };

        let (black, red) = panel_bytes.ok_or_else(|| {
            JsValue::from_str(&format!(
                "SSD1680 device not found on SPI peripheral '{}'",
                binding.peripheral
            ))
        })?;
        let mut combined = Vec::with_capacity(black.len() + red.len());
        combined.extend_from_slice(&black);
        combined.extend_from_slice(&red);
        Ok(combined.into_boxed_slice())
    }

    /// Cheap accessor returning just the SSD1680 refresh-generation counter.
    /// UI uses this to decide whether to re-fetch the (larger) framebuffer.
    #[wasm_bindgen]
    pub fn get_ssd1680_refresh_generation(&self, device_id: &str) -> Result<u32, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| {
                b.id == device_id
                    && matches!(
                        b.device_type.as_deref(),
                        Some("ssd1680_tricolor_290") | Some("epd-2in9-tricolor")
                    )
            })
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "No ssd1680_tricolor_290 board_io binding '{}'",
                    device_id
                ))
            })?;

        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "SPI peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;

        let any = machine.bus.peripherals[idx]
            .dev
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        let gen = if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ssd1680Tricolor290>()
                }).map(|panel| panel.refresh_generation())
            })
        } else if let Some(spi) =
            any.downcast_ref::<labwired_core::peripherals::esp32::spi::Esp32Spi>()
        {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ssd1680Tricolor290>()
                }).map(|panel| panel.refresh_generation())
            })
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an SPI controller",
                binding.peripheral
            )));
        };
        gen.ok_or_else(|| {
            JsValue::from_str(&format!(
                "SSD1680 device not found on SPI peripheral '{}'",
                binding.peripheral
            ))
        })
    }

    /// Same shape as [`get_ssd1680_framebuffer`] but for the UC8151D-family
    /// tri-color panel attached by [`install_arduino_esp32_quirks`]. The
    /// board_io binding type may say `ssd1680_tricolor_290` (since system
    /// YAMLs were authored before the UC8151D split); we ignore that and
    /// just find a `Uc8151dTricolor290` on the named SPI peripheral.
    #[wasm_bindgen]
    pub fn get_uc8151d_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        use labwired_core::peripherals::components::Uc8151dTricolor290;
        use labwired_core::peripherals::esp32::spi::Esp32Spi;
        let machine = self.machine.as_ref().unwrap();
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id)
            .ok_or_else(|| JsValue::from_str(&format!("No board_io binding '{}'", device_id)))?;
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "SPI peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;
        let panel_bytes =
            if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
                spi.attached_devices.iter().find_map(|dev| {
                    dev.as_any()
                        .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
                        .map(|p| (p.black_plane().to_vec(), p.red_plane().to_vec()))
                })
            } else if let Some(spi) = any.downcast_ref::<Esp32Spi>() {
                spi.attached_devices.iter().find_map(|dev| {
                    dev.as_any()
                        .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
                        .map(|p| (p.black_plane().to_vec(), p.red_plane().to_vec()))
                })
            } else {
                return Err(JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                )));
            };
        let (black, red) = panel_bytes.ok_or_else(|| {
            JsValue::from_str(&format!(
                "UC8151D device not found on SPI peripheral '{}'",
                binding.peripheral
            ))
        })?;
        let mut combined = Vec::with_capacity(black.len() + red.len());
        combined.extend_from_slice(&black);
        combined.extend_from_slice(&red);
        Ok(combined.into_boxed_slice())
    }

    /// Cheap accessor returning just the UC8151D refresh-generation counter.
    #[wasm_bindgen]
    pub fn get_uc8151d_refresh_generation(&self, device_id: &str) -> Result<u32, JsValue> {
        use labwired_core::peripherals::components::Uc8151dTricolor290;
        use labwired_core::peripherals::esp32::spi::Esp32Spi;
        let machine = self.machine.as_ref().unwrap();
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id)
            .ok_or_else(|| JsValue::from_str(&format!("No board_io binding '{}'", device_id)))?;
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "SPI peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;
        let gen = if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any()
                    .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
                    .map(|p| p.refresh_generation())
            })
        } else if let Some(spi) = any.downcast_ref::<Esp32Spi>() {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any()
                    .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
                    .map(|p| p.refresh_generation())
            })
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an SPI controller",
                binding.peripheral
            )));
        };
        gen.ok_or_else(|| {
            JsValue::from_str(&format!(
                "UC8151D device not found on SPI peripheral '{}'",
                binding.peripheral
            ))
        })
    }

    /// Read back the current state of each SPI sensor declared in `board_io`.
    /// Returns `[{ id, kind: "max31855", tc_c, internal_c }, ...]`.
    #[wasm_bindgen]
    pub fn get_spi_device_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "max31855" => t,
                _ => continue,
            };
            let Some(idx) = machine
                .bus
                .find_peripheral_index_by_name(&binding.peripheral)
            else {
                continue;
            };
            let Some(any) = machine.bus.peripherals[idx].dev.as_any() else {
                continue;
            };
            let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() else {
                continue;
            };

            if device_type == "max31855" {
                for device in &spi.attached_devices {
                    if let Some(sensor) = device.as_any().and_then(|a| {
                        a.downcast_ref::<labwired_core::peripherals::components::Max31855>()
                    }) {
                        let (tc_c, internal_c) = sensor.temperature();
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "max31855",
                            "tc_c": tc_c,
                            "internal_c": internal_c,
                        }));
                        break;
                    }
                }
            }
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Set the simulated thermocouple and internal temperatures on a MAX31855 device.
    #[wasm_bindgen]
    pub fn set_max31855_temperature(
        &mut self,
        device_id: &str,
        tc_c: f32,
        internal_c: f32,
    ) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("max31855"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!("No MAX31855 board_io binding '{}'", device_id))
            })?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "SPI peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("SPI peripheral does not support downcasting"))?;
        let spi = any
            .downcast_mut::<labwired_core::peripherals::spi::Spi>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                ))
            })?;

        for device in &mut spi.attached_devices {
            if let Some(sensor) = device
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<labwired_core::peripherals::components::Max31855>())
            {
                sensor.set_temperature(tc_c, internal_c);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "MAX31855 device not found on '{}'",
            binding.peripheral
        )))
    }

    /// Set the simulated position on a NEO-6M GPS module attached to a UART peripheral.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "neo6m-gps"`.
    #[wasm_bindgen]
    pub fn set_gps_position(&mut self, device_id: &str, lat: f64, lon: f64) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("neo6m-gps"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!("No neo6m-gps board_io binding '{}'", device_id))
            })?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "UART peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("UART peripheral does not support downcasting"))?;
        let uart = any
            .downcast_mut::<labwired_core::peripherals::uart::Uart>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not a UART controller",
                    binding.peripheral
                ))
            })?;

        for stream in &mut uart.attached_streams {
            if let Some(gps) = stream
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<labwired_core::peripherals::components::Neo6mGps>())
            {
                gps.set_position(lat, lon);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "Neo6mGps not found on UART '{}'",
            binding.peripheral
        )))
    }

    /// Enable or disable the GPS fix on a NEO-6M module.
    #[wasm_bindgen]
    pub fn set_gps_fix(&mut self, device_id: &str, active: bool) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("neo6m-gps"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!("No neo6m-gps board_io binding '{}'", device_id))
            })?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "UART peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("UART peripheral does not support downcasting"))?;
        let uart = any
            .downcast_mut::<labwired_core::peripherals::uart::Uart>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not a UART controller",
                    binding.peripheral
                ))
            })?;

        for stream in &mut uart.attached_streams {
            if let Some(gps) = stream
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<labwired_core::peripherals::components::Neo6mGps>())
            {
                gps.set_fix(active);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "Neo6mGps not found on UART '{}'",
            binding.peripheral
        )))
    }

    /// Read back the current state of all NEO-6M GPS devices declared in `board_io`.
    /// Returns `[{ id, kind: "neo6m-gps", lat, lon, has_fix }]`.
    #[wasm_bindgen]
    pub fn get_uart_device_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "neo6m-gps" => t,
                _ => continue,
            };
            let Some(idx) = machine
                .bus
                .find_peripheral_index_by_name(&binding.peripheral)
            else {
                continue;
            };
            let Some(any) = machine.bus.peripherals[idx].dev.as_any() else {
                continue;
            };
            let Some(uart) = any.downcast_ref::<labwired_core::peripherals::uart::Uart>() else {
                continue;
            };

            if device_type == "neo6m-gps" {
                for stream in &uart.attached_streams {
                    if let Some(gps) = stream.as_any().and_then(|a| {
                        a.downcast_ref::<labwired_core::peripherals::components::Neo6mGps>()
                    }) {
                        let (lat, lon) = gps.position();
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "neo6m-gps",
                            "lat": lat,
                            "lon": lon,
                            "has_fix": gps.has_fix(),
                        }));
                        break;
                    }
                }
            }
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// List all peripherals: [{ name, base_address }]
    #[wasm_bindgen]
    pub fn get_peripheral_list(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let list: Vec<serde_json::Value> = machine
            .bus
            .peripherals
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "base_address": format!("0x{:08X}", p.base),
                })
            })
            .collect();
        serde_wasm_bindgen::to_value(&list).unwrap_or(JsValue::NULL)
    }

    // ──────────────────────────────────────────────────────────────────────
    //  IO-Link DI demo: 74HC165 input toggling + IO-Link master readout.
    //  These find the device by iterating the bus (the shifter/master are
    //  `external_devices`, not `board_io` bindings), which suits the single
    //  shifter + single master of the AL2205-style demo.
    // ──────────────────────────────────────────────────────────────────────

    /// Set all 8 digital inputs of the 74HC165 shift register at once
    /// (bit `i` = channel `i`). Returns an error if no shifter is wired.
    #[wasm_bindgen]
    pub fn set_sn74hc165_inputs(&mut self, value: u8) -> Result<(), JsValue> {
        let machine = self.machine.as_mut().unwrap();
        for p in &mut machine.bus.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(spi) = any.downcast_mut::<labwired_core::peripherals::spi::Spi>() else {
                continue;
            };
            for device in &mut spi.attached_devices {
                if let Some(sr) = device.as_any_mut().and_then(|a| {
                    a.downcast_mut::<labwired_core::peripherals::components::Sn74hc165>()
                }) {
                    sr.set_inputs(value);
                    return Ok(());
                }
            }
        }
        Err(JsValue::from_str("no 74HC165 shift register attached"))
    }

    /// Read the 74HC165's live input byte (bit `i` = channel `i`), or `-1` if
    /// no shifter is wired. Lets the UI reflect the device's real state rather
    /// than tracking it in JS.
    #[wasm_bindgen]
    pub fn get_sn74hc165_inputs(&self) -> i32 {
        let machine = self.machine.as_ref().unwrap();
        for p in &machine.bus.peripherals {
            let Some(any) = p.dev.as_any() else {
                continue;
            };
            let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() else {
                continue;
            };
            for device in &spi.attached_devices {
                if let Some(sr) = device.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Sn74hc165>()
                }) {
                    return sr.inputs() as i32;
                }
            }
        }
        -1
    }

    /// Toggle a single 74HC165 input channel (0..=7) high or low.
    #[wasm_bindgen]
    pub fn set_sn74hc165_channel(&mut self, channel: u8, high: bool) -> Result<(), JsValue> {
        let machine = self.machine.as_mut().unwrap();
        for p in &mut machine.bus.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(spi) = any.downcast_mut::<labwired_core::peripherals::spi::Spi>() else {
                continue;
            };
            for device in &mut spi.attached_devices {
                if let Some(sr) = device.as_any_mut().and_then(|a| {
                    a.downcast_mut::<labwired_core::peripherals::components::Sn74hc165>()
                }) {
                    sr.set_channel(channel, high);
                    return Ok(());
                }
            }
        }
        Err(JsValue::from_str("no 74HC165 shift register attached"))
    }

    /// Read the IO-Link master peer's live state: `{ link_state, pd_valid,
    /// input_byte }`. Returns `null` if no master is wired.
    #[wasm_bindgen]
    pub fn get_iolink_master_state(&self) -> JsValue {
        use labwired_core::peripherals::components::{IolinkLinkState, IolinkMaster};
        let machine = self.machine.as_ref().unwrap();
        for p in &machine.bus.peripherals {
            let Some(any) = p.dev.as_any() else {
                continue;
            };
            let Some(uart) = any.downcast_ref::<labwired_core::peripherals::uart::Uart>() else {
                continue;
            };
            for stream in &uart.attached_streams {
                if let Some(m) = stream
                    .as_any()
                    .and_then(|a| a.downcast_ref::<IolinkMaster>())
                {
                    let link = match m.link_state {
                        IolinkLinkState::Startup => "startup",
                        IolinkLinkState::Operate => "operate",
                    };
                    let v = serde_json::json!({
                        "link_state": link,
                        "pd_valid": m.pd_valid,
                        "input_byte": m.input_byte(),
                    });
                    return serde_wasm_bindgen::to_value(&v).unwrap_or(JsValue::NULL);
                }
            }
        }
        JsValue::NULL
    }

    /// Snapshot of the IO-Link master's captured transactions (oldest→newest),
    /// for the IO-Link Analyzer instrument. Empty array if no master is wired.
    #[wasm_bindgen]
    pub fn iolink_trace_snapshot(&self) -> JsValue {
        use labwired_core::peripherals::components::IolinkMaster;
        let Some(machine) = self.machine.as_ref() else {
            return serde_wasm_bindgen::to_value(&Vec::<
                labwired_core::peripherals::components::IolinkXfer,
            >::new())
            .unwrap_or(JsValue::NULL);
        };
        for p in &machine.bus.peripherals {
            let Some(any) = p.dev.as_any() else { continue };
            let Some(uart) = any.downcast_ref::<labwired_core::peripherals::uart::Uart>() else {
                continue;
            };
            for stream in &uart.attached_streams {
                if let Some(m) = stream
                    .as_any()
                    .and_then(|a| a.downcast_ref::<IolinkMaster>())
                {
                    let trace = m.trace_snapshot();
                    return serde_wasm_bindgen::to_value(&trace).unwrap_or(JsValue::NULL);
                }
            }
        }
        serde_wasm_bindgen::to_value(
            &Vec::<labwired_core::peripherals::components::IolinkXfer>::new(),
        )
        .unwrap_or(JsValue::NULL)
    }

    /// Clear the IO-Link master's trace ring.
    #[wasm_bindgen]
    pub fn iolink_trace_clear(&mut self) {
        use labwired_core::peripherals::components::IolinkMaster;
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        for p in &mut machine.bus.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(uart) = any.downcast_mut::<labwired_core::peripherals::uart::Uart>() else {
                continue;
            };
            for stream in &mut uart.attached_streams {
                if let Some(m) = stream
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<IolinkMaster>())
                {
                    m.trace_clear();
                    return;
                }
            }
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    //  Arduino-ESP32 bootstrap glue. Call after constructing the WasmSimulator
    //  with an ESP32-classic manifest + an Arduino-ESP32 firmware ELF (e.g.
    //  the reference firmware). Bakes in:
    //    * Memory pre-fakes (partition header, RTC freq probe, dual-core
    //      handshake bytes, ROM data region).
    //    * Flash thunks for the ESP-IDF + Arduino-ESP32 functions whose real
    //      behavior our sim can't model (heap_caps_*, esp_timer_init, locks,
    //      setCpuFrequencyMhz, esp_ota_get_running_partition, HardwareSerial,
    //      delay, WifiWsLink::begin, gpio_matrix_in/out, etc).
    //    * One-byte runtime patch at 0x400E_90DE so loopTask gets pinned
    //      to PRO_CPU instead of the APP_CPU we don't emulate.
    //    * Enables the IPI bridge state so `step_with_esp32_aids` raises
    //      INTERRUPT bits when firmware writes DPORT_CPU_INTR_FROM_CPU.
    // ──────────────────────────────────────────────────────────────────────
    #[wasm_bindgen]
    pub fn install_esp32_arduino_quirks(&mut self) -> Result<(), JsValue> {
        use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("no machine"))?;

        // Seed SP — call_start_cpu0 expects BROM to have placed SP near top
        // of DRAM (0x3FFE_0000). We skip BROM, so do it ourselves.
        machine.cpu.set_sp(0x3FFE_0000);
        // Force loopTask onto PRO_CPU.
        let _ = machine.bus.write_u8(0x400E_90DE, 0x08);

        // Dual-core handshake fakes (single-CPU sim).
        let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01); // s_cpu_up[1]
        let _ = machine.bus.write_u8(0x3FFC_6F01, 0x01); // s_cpu_inited[0]
        let _ = machine.bus.write_u8(0x3FFC_6F02, 0x01); // s_cpu_inited[1]
        let _ = machine.bus.write_u8(0x3FFC_6FFD, 0x01); // s_system_inited[0]
        let _ = machine.bus.write_u8(0x3FFC_6FFE, 0x01); // s_system_inited[1]
        let _ = machine.bus.write_u8(0x3FFC_7190, 0x01); // s_other_cpu_startup_done

        // RTC_APB_FREQ_REG (0x3FF4_80B0) is now pre-seeded with the 40 MHz
        // encoding (0x0050_0050) by the RtcCntl peripheral at construction —
        // no quirk write needed here.

        // Fake esp_image_header_t at 0x3F40_0000 (24 bytes), entry = the reference firmware.
        let entry = 0x40081bf0_u32;
        let header: [u8; 24] = [
            0xE9,
            0x01,
            0x00,
            0x00,
            (entry & 0xFF) as u8,
            ((entry >> 8) & 0xFF) as u8,
            ((entry >> 16) & 0xFF) as u8,
            ((entry >> 24) & 0xFF) as u8,
            0xEE,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        for (i, &b) in header.iter().enumerate() {
            let _ = machine.bus.write_u8(0x3F40_0000 + i as u64, b);
        }

        // Flash thunks. Addresses are the reference firmware-firmware-specific (PC of the
        // function in the ELF) — see crates/core/tests/e2e_external_arduino_esp32_in_sim.rs
        // for the same list with detailed per-thunk rationale.
        let thunks: &[(u32, rom_thunks::RomThunkFn)] = &[
            (0x400e_e3b0, rom_thunks::esp_idf_heap_caps_init),
            (0x4008_2904, rom_thunks::esp_idf_heap_caps_malloc),
            (0x4008_2a70, rom_thunks::esp_idf_heap_caps_calloc),
            (0x4008_25dc, rom_thunks::esp_idf_heap_caps_free),
            (0x4008_29f0, rom_thunks::esp_idf_heap_caps_realloc),
            (0x4012_9034, rom_thunks::nop_return_zero), // esp_timer_init
            (0x4008_17dc, rom_thunks::nop_return_zero), // spi_flash_disable_...
            (0x4008_188c, rom_thunks::nop_return_zero), // spi_flash_enable_...
            (0x4008_3384, rom_thunks::nop_return_zero), // __retarget_lock_init_recursive
            (0x4008_339c, rom_thunks::nop_return_zero), // __retarget_lock_close_recursive
            (0x4008_33b0, rom_thunks::nop_return_zero), // __retarget_lock_acquire_recursive
            (0x4008_33cc, rom_thunks::nop_return_zero), // __retarget_lock_release_recursive
            (0x4008_bbd0, rom_thunks::nop_return_zero), // _esp_error_check_failed
            (0x400e_99dc, rom_thunks::nop_return_zero), // setCpuFrequencyMhz
            (0x400e_ae18, rom_thunks::nop_return_fake_ptr), // esp_ota_get_running_partition
            (0x400e_2280, rom_thunks::nop_return_zero), // HardwareSerial::begin
            (0x400e_5c28, rom_thunks::nop_return_zero), // Arduino delay()
            (0x400d_de98, rom_thunks::nop_return_zero), // WifiWsLink::begin
            (0x400d_dccc, rom_thunks::nop_return_zero), // WifiWsLink::loop
            (0x400e_0034, rom_thunks::nop_return_zero), // anon-ns sendHello
        ];
        for &(pc, f) in thunks {
            machine
                .bus
                .install_flash_thunk(pc, f)
                .map_err(|e| JsValue::from_str(&format!("install thunk @{pc:#x}: {e}")))?;
        }

        self.esp32_ipi = Some(Esp32IpiBridge::default());
        Ok(())
    }

    /// Auto-discovery counterpart to [`Self::install_esp32_arduino_quirks`].
    ///
    /// Mirrors the CLI's `arduino-esp32` snapshot-capture profile —
    /// resolves Arduino-ESP32 thunk PCs from the ELF symbol table instead
    /// of hand-curated hardcoded addresses. Works for any GxEPD2-class
    /// sketch (labwired-ereader, future user sketches) without needing
    /// to know its binary layout in advance.
    ///
    /// Caller must pass the same ELF bytes that were loaded via
    /// `load_firmware`. The thunks are installed as flash patches over
    /// the resolved PCs; calling this without the matching ELF is a no-op
    /// (symbols don't resolve → no thunks installed).
    ///
    /// Also attaches a `Uc8151dTricolor290` panel to spi3 (the SSD1680
    /// panel attached by default doesn't decode UC8151D opcodes
    /// `0x00 PSR` / `0x04 PON` / `0x10 DTM1` / `0x12 DRF` / `0x13 DTM2`
    /// that GxEPD2_290_C90c / Z13c emits).
    #[wasm_bindgen]
    pub fn install_arduino_esp32_quirks(&mut self, elf_bytes: &[u8]) -> Result<(), JsValue> {
        use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("no machine"))?;

        // NO hardcoded peripheral here. The panel (and any other external device)
        // is attached from the board manifest by attach_esp32_external_devices
        // during system load — the single source of truth for peripheral wiring,
        // model, CS and DC pins. This method only installs the firmware-boot
        // thunks + CPU seed below.

        // Seed SP — call_start_cpu0 expects BROM to have placed SP near
        // top of DRAM. We skip BROM.
        machine.cpu.set_sp(0x3FFE_0000);
        // RTC_APB_FREQ_REG (0x3FF4_80B0) now comes pre-seeded with the 40 MHz
        // encoding (0x0050_0050) from the RtcCntl peripheral — no quirk
        // write needed.

        let symbol_addrs = labwired_loader::extract_arduino_esp32_thunks(elf_bytes);

        // Dual-core handshake bytes — resolved per firmware. Recorded for
        // the keep-alive in step_with_esp32_aids so the firmware's `.bss`
        // zero-init (which runs after this install but before the spin-wait
        // check in call_start_cpu0) can't wipe them.
        let mut handshake_bytes: Vec<u32> = Vec::new();
        for sym in &[
            "s_resume_cores",
            "s_cpu_up",
            "s_cpu_inited",
            "s_system_inited",
        ] {
            if let Some(&addr) = symbol_addrs.get(*sym) {
                let _ = machine.bus.write_u8(addr as u64, 0x01);
                let _ = machine.bus.write_u8(addr as u64 + 1, 0x01);
                handshake_bytes.push(addr);
                handshake_bytes.push(addr + 1);
            }
        }
        if let Some(&addr) = symbol_addrs.get("s_other_cpu_startup_done") {
            let _ = machine.bus.write_u8(addr as u64, 0x01);
            handshake_bytes.push(addr);
        }
        // Re-assert these flags the instant PRO_CPU releases APP_CPU, so
        // newer arduino-esp32 cores whose `start_other_core` spin-waits
        // with a tight cycle-count timeout see APP_CPU "up" without
        // depending on the coarse 10k-cycle keep-alive in
        // step_with_esp32_aids. Models APP_CPU bring-up; see
        // labwired_core rom_thunks::ets_set_appcpu_boot_addr.
        labwired_core::peripherals::esp_xtensa_common::rom_thunks::set_appcpu_up_flags(
            handshake_bytes.clone(),
        );

        // loopTask xCoreID patch — repin loopTask from APP_CPU to PRO_CPU
        // (we model only PRO_CPU). Handles both the legacy and IDF-5.x
        // app_main layouts. See rom_thunks::repin_loop_task.
        if let Some(&app_main_addr) = symbol_addrs.get("app_main") {
            let _ = rom_thunks::repin_loop_task(&mut machine.bus, app_main_addr);
        }

        // pxCurrentTCB pointer seed for xTaskGetCurrentTaskHandle thunk.
        if let Some(&addr) = symbol_addrs.get("pxCurrentTCB") {
            rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(addr)));
        }

        // Build the thunk list — by-symbol lookups, skip when symbol
        // missing (sketch doesn't import that function).
        let mut thunks: Vec<(u32, rom_thunks::RomThunkFn)> = Vec::new();
        let push_named = |list: &mut Vec<(u32, rom_thunks::RomThunkFn)>,
                          sym: &str,
                          f: rom_thunks::RomThunkFn| {
            if let Some(&pc) = symbol_addrs.get(sym) {
                list.push((pc, f));
            }
        };

        push_named(
            &mut thunks,
            "heap_caps_init",
            rom_thunks::esp_idf_heap_caps_init,
        );
        push_named(
            &mut thunks,
            "heap_caps_malloc",
            rom_thunks::esp_idf_heap_caps_malloc,
        );
        push_named(
            &mut thunks,
            "heap_caps_calloc",
            rom_thunks::esp_idf_heap_caps_calloc,
        );
        push_named(
            &mut thunks,
            "heap_caps_free",
            rom_thunks::esp_idf_heap_caps_free,
        );
        push_named(
            &mut thunks,
            "heap_caps_realloc",
            rom_thunks::esp_idf_heap_caps_realloc,
        );

        // No-op stubs for ESP-IDF / Arduino-ESP32 init paths we don't model.
        for sym in &[
            "esp_timer_init",
            "spi_flash_disable_interrupts_caches_and_other_cpu",
            "spi_flash_enable_interrupts_caches_and_other_cpu",
            "__retarget_lock_init_recursive",
            "__retarget_lock_close_recursive",
            "__retarget_lock_acquire_recursive",
            "__retarget_lock_release_recursive",
            "_esp_error_check_failed",
            "setCpuFrequencyMhz",
            "delay",
            "xQueueGiveMutexRecursive",
            "xQueueTakeMutexRecursive",
            "esp_ipc_init",
            "esp_ipc_isr_init",
            "esp_log_impl_lock",
            "esp_log_impl_lock_timeout",
            "esp_log_impl_unlock",
            "esp_panic_handler",
            "esp_panic_handler_reconfigure_wdts",
            "__assert_func",
            "__assert",
            "abort",
            "pthread_key_create",
            "pthread_setspecific",
            "pthread_getspecific",
            "pthread_mutex_init",
            "pthread_mutex_lock",
            "pthread_mutex_unlock",
            "_lock_acquire",
            "_lock_acquire_recursive",
            "_lock_release",
            "_lock_release_recursive",
            "_lock_init",
            "_lock_init_recursive",
            "_lock_close",
            "_lock_close_recursive",
            "_lock_try_acquire",
            "_lock_try_acquire_recursive",
            "esp_pthread_init",
            "esp_task_wdt_reset",
            "esp_task_wdt_init",
            "esp_task_wdt_add",
            "esp_task_wdt_delete",
            "esp_clk_init",
            "esp_perip_clk_init",
            "core_intr_matrix_clear",
            "esp_flash_init",
            "esp_flash_init_default_chip",
            "esp_flash_init_main",
            "esp_flash_app_init",
            "esp_flash_app_enable_os_functions",
            "esp_flash_app_disable_protect",
            "esp_flash_app_disable_os_functions",
            "esp_flash_read_chip_id",
            "esp_flash_chip_driver_initialized",
            "do_core_init",
            "do_secondary_init",
            // NB: `esp_startup_start_app` is intentionally NOT stubbed —
            // its real impl calls `vTaskStartScheduler()` which never
            // returns. Stubbing makes `start_cpu0` fall into the `j .`
            // safety-loop at its tail and the FreeRTOS scheduler never
            // takes over (loopTask / setup() never run). Required for the
            // labwired-ereader Arduino sketch to actually paint.
            "esp_partition_main_flash_region_safe",
            "spi_flash_init",
            "spi_flash_init_chip_state",
            "esp_efuse_check_errors",
            "esp_dport_access_stall_other_cpu_start",
            "esp_dport_access_stall_other_cpu_end",
            "esp_cpu_unstall",
            "bootloader_flash_update_id",
            "bootloader_init_mem",
            "esp_mspi_pin_init",
            "spi_flash_init_chip_state",
            "esp_log_timestamp",
            "esp_log_early_timestamp",
            "esp_log_writev",
            "esp_random",
            "esp_fill_random",
            "_ZN14HardwareSerial5writeEh",
            "_ZN14HardwareSerial5writeEPKhj",
            "_ZN14HardwareSerial9availableEv",
            "_ZN14HardwareSerial5flushEv",
            "_ZN14HardwareSerial9readBytesEPcj",
            "_ZN14HardwareSerial9readBytesEPhj",
            // HardwareSerial::begin — Arduino-ESP32's serial init walks
            // through _get_effective_baudrate which divides by
            // getApbFrequency(). Our sim returns 0 → divide-by-zero
            // exception. Skip the whole begin() rather than emulate the
            // baud calculation; we don't model UART output anyway.
            "_ZN14HardwareSerial5beginEmjaabmh",
            "_get_effective_baudrate",
            "uartAvailable",
            "uartAvailableForWrite",
            "uartWrite",
            "uartWriteBuf",
            "_Z14serialEventRunv",
            "vListInsert",
        ] {
            push_named(&mut thunks, sym, rom_thunks::nop_return_zero);
        }

        // Functions that need real returns / args.
        push_named(
            &mut thunks,
            "esp_ota_get_running_partition",
            rom_thunks::nop_return_fake_ptr,
        );
        // Return a non-NULL fake handle so callers' `assert(mutex != NULL)`
        // passes. Mutex semantics aren't modeled — the firmware will treat
        // the returned pointer as opaque and pass it to xSemaphoreTake/Give
        // which are already stubbed to "success".
        for sym in &[
            "xQueueCreateMutex",
            "xQueueCreateMutexStatic",
            "xQueueGenericCreate",
            "xSemaphoreCreateMutex",
            "xSemaphoreCreateBinary",
            "xSemaphoreCreateCounting",
            "xQueueCreateCountingSemaphore",
            "xEventGroupCreate",
        ] {
            push_named(&mut thunks, sym, rom_thunks::nop_return_fake_ptr);
        }
        // Stub spi_flash_init_lock — the real impl creates a mutex via
        // xSemaphoreCreateMutex and asserts non-NULL; we don't need real
        // flash-op locking in the single-task sim.
        for sym in &[
            "spi_flash_init_lock",
            "spi_flash_op_lock",
            "spi_flash_op_unlock",
        ] {
            push_named(&mut thunks, sym, rom_thunks::nop_return_zero);
        }
        push_named(&mut thunks, "esp_chip_info", rom_thunks::esp_chip_info_stub);
        push_named(
            &mut thunks,
            "__getreent",
            rom_thunks::getreent_dram_fake_ptr,
        );
        push_named(
            &mut thunks,
            "esp_timer_impl_get_counter_reg",
            rom_thunks::monotonic_counter_32,
        );
        push_named(
            &mut thunks,
            "esp_clk_cpu_freq",
            rom_thunks::esp_clk_cpu_freq_240mhz,
        );
        push_named(
            &mut thunks,
            "xQueueCreateMutexStatic",
            rom_thunks::x_queue_create_mutex_static_echo,
        );
        push_named(
            &mut thunks,
            "xTaskGetCurrentTaskHandle",
            rom_thunks::x_task_get_current_task_handle,
        );
        push_named(
            &mut thunks,
            "xQueueSemaphoreTake",
            rom_thunks::return_pd_true,
        );
        push_named(&mut thunks, "xQueueGenericSend", rom_thunks::return_pd_true);
        push_named(
            &mut thunks,
            "ulTaskGenericNotifyTake",
            rom_thunks::return_pd_true,
        );
        push_named(&mut thunks, "spiStartBus", rom_thunks::spi_start_bus_fake);
        push_named(
            &mut thunks,
            "_ZN8SPIClass16beginTransactionE11SPISettings",
            rom_thunks::spi_class_begin_transaction,
        );

        // No GxEPD2 cmd/data bypass. The real compiled _writeCommand/_writeData
        // run: digitalWrite(DC=GPIO17) → SPI.transfer → spiTransferByteNL writes
        // the SPI3 FIFO/MOSI_DLEN/CMD.USR registers, and Esp32Spi drains the byte
        // to the panel framed by the latched DC GPIO. Bytes reach the panel
        // through real register machinery (verified against the real firmware.elf
        // in tests/e2e_labwired_ereader.rs: 431 SPI3 transactions → refresh).

        // xthal_window_spill_nw — semantic spill via shadow stack. Only the
        // `_nw` leaf is thunked; the `xthal_window_spill` wrapper is a thin
        // CALL{n}-entered PS-save shell that must run its real
        // `entry / call0 _nw / retw` natively — thunking it returns via a0
        // (the caller's return addr, since the wrapper's clobbered ENTRY
        // never set up a0), faulting in the first-task dispatch.
        push_named(
            &mut thunks,
            "xthal_window_spill_nw",
            rom_thunks::xthal_window_spill_thunk,
        );

        // Real-silicon noreturn — abort_halt prints diagnostics and
        // halts the CPU rather than returning, to avoid tight
        // assert→return loops.
        for sym in &[
            "panic_abort",
            "__assert_func",
            "abort",
            "__assert",
            "__cxa_pure_virtual",
            "__cxa_throw",
        ] {
            push_named(&mut thunks, sym, rom_thunks::abort_halt);
        }

        for (pc, f) in thunks {
            machine
                .bus
                .install_flash_thunk(pc, f)
                .map_err(|e| JsValue::from_str(&format!("install thunk @{pc:#x}: {e}")))?;
        }

        self.esp32_ipi = Some(Esp32IpiBridge {
            handshake_bytes,
            ..Esp32IpiBridge::default()
        });
        Ok(())
    }

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
    pub fn apply_agentdeck_quirks(&mut self) -> Result<(), JsValue> {
        #[allow(deprecated)]
        self.install_esp32_arduino_quirks()
    }

    /// Apply a binary `MachineRuntimeSnapshot` (LWRS-framed bincode blob,
    /// produced by `labwired-cli snapshot capture` or `Machine::take_runtime_snapshot`)
    /// to the currently-loaded machine. Bypasses the cold boot — the firmware
    /// resumes mid-flight from the captured CPU + peripheral state.
    ///
    /// Must be called after firmware has been loaded onto the same system
    /// manifest (peripheral names + CPU arch must match the snapshot). On
    /// mismatch the call returns an error and the machine state is left
    /// partially overwritten — callers should treat that as a hard reset.
    #[wasm_bindgen]
    pub fn apply_runtime_snapshot(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("no machine"))?;
        let snap = labwired_core::runtime_snapshot::MachineRuntimeSnapshot::from_bytes(bytes)
            .map_err(|e| JsValue::from_str(&format!("snapshot decode: {e}")))?;
        machine
            .apply_runtime_snapshot(&snap)
            .map_err(|e| JsValue::from_str(&format!("snapshot apply: {e}")))?;
        Ok(())
    }

    /// Capture the current machine state as a binary `MachineRuntimeSnapshot`
    /// (LWRS-framed bincode blob). Mirror of `apply_runtime_snapshot` —
    /// returned bytes can be fed back to `apply_runtime_snapshot` on a fresh
    /// `WasmSimulator` with the same firmware + bus topology.
    #[wasm_bindgen]
    pub fn take_runtime_snapshot(&self) -> Result<Vec<u8>, JsValue> {
        let machine = self
            .machine
            .as_ref()
            .ok_or_else(|| JsValue::from_str("no machine"))?;
        Ok(machine.take_runtime_snapshot().to_bytes())
    }

    /// Re-write the dual-core handshake bytes. Call every ~10k steps from JS
    /// — firmware boot code revisits these and we need them to stay 1.
    #[wasm_bindgen]
    pub fn keep_alive_esp32_dual_core(&mut self) {
        let machine = match self.machine.as_mut() {
            Some(m) => m,
            None => return,
        };
        let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_6F01, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_6F02, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_6FFD, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_6FFE, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_7190, 0x01);
    }

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

// WasmGdbEventLoop removed — see `gdb_process_packet` above for the rationale.
// Restoring this requires `LabwiredTarget` to be implemented for an arch-erased
// CPU type, which is the follow-up tracked alongside Phase 1.
