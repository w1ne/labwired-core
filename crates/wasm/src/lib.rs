use labwired_config::{BoardIoBinding, BoardIoKind, BoardIoSignal, ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::decoder::arm::{decode_thumb_16, decode_thumb_32};
use labwired_core::memory::LinearMemory;
use labwired_core::peripherals::adc::Adc;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::{Cpu, Machine};
use labwired_loader::load_elf_bytes;
use wasm_bindgen::prelude::*;

use gdbstub::conn::{Connection, ConnectionExt};
use gdbstub::stub::{BaseStopReason, GdbStub};
use labwired_gdbstub::LabwiredTarget;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct WasmGdbError;
impl std::fmt::Display for WasmGdbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WasmGdbError")
    }
}
impl std::error::Error for WasmGdbError {}

// Thread-local output buffer that WasmGdbConn writes into, allowing retrieval after
// GdbStub consumes the connection.
thread_local! {
    static GDB_OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

struct WasmGdbConn {
    input: VecDeque<u8>,
    peeked: Option<u8>,
}

impl WasmGdbConn {
    fn new(packet: &[u8]) -> Self {
        GDB_OUTPUT.with(|o| o.borrow_mut().clear());
        WasmGdbConn {
            input: packet.iter().copied().collect(),
            peeked: None,
        }
    }
}

impl Connection for WasmGdbConn {
    type Error = WasmGdbError;
    fn write(&mut self, byte: u8) -> Result<(), Self::Error> {
        GDB_OUTPUT.with(|o| o.borrow_mut().push(byte));
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ConnectionExt for WasmGdbConn {
    fn read(&mut self) -> Result<u8, Self::Error> {
        if let Some(b) = self.peeked.take() {
            return Ok(b);
        }
        self.input.pop_front().ok_or(WasmGdbError)
    }

    fn peek(&mut self) -> Result<Option<u8>, Self::Error> {
        if self.peeked.is_none() {
            self.peeked = self.input.front().copied();
        }
        Ok(self.peeked)
    }
}

#[wasm_bindgen]
pub struct WasmSimulator {
    machine: Option<Machine<CortexM>>,
    board_io: Vec<BoardIoBinding>,
    uart_sink: Arc<Mutex<Vec<u8>>>,
    uart_rx_bufs: Vec<Arc<Mutex<VecDeque<u8>>>>,
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
        let mut machine = Machine::new(cpu, bus);

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
        })
    }

    /// Config-driven constructor: initialize from system YAML, chip YAML, and firmware ELF.
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

        let mut bus = SystemBus::from_config(&chip, &manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(uart_sink.clone(), false);
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);

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
        })
    }

    fn machine(&mut self) -> &mut Machine<CortexM> {
        self.machine.as_mut().unwrap()
    }

    /// Read the output state of a board_io binding using peripheral snapshot.
    fn read_board_io_state(&self, machine: &Machine<CortexM>, binding: &BoardIoBinding) -> bool {
        let idx = match machine.bus.find_peripheral_index_by_name(&binding.peripheral) {
            Some(i) => i,
            None => return false,
        };

        let snapshot = machine.bus.peripherals[idx].dev.snapshot();

        let register = match binding.kind {
            BoardIoKind::Led | BoardIoKind::PwmOutput => "odr",
            BoardIoKind::Button => "idr",
            // Analog/bus kinds are not boolean — handled by get_board_io_analog_states
            BoardIoKind::AdcInput | BoardIoKind::I2cDevice | BoardIoKind::SpiDevice => {
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

    #[wasm_bindgen]
    pub fn gdb_process_packet(&mut self, packet: &[u8]) -> Vec<u8> {
        // Take the machine out of self and put it into a target
        let machine = self.machine.take().unwrap();
        let mut target = LabwiredTarget::new(machine);

        let conn = WasmGdbConn::new(packet);
        let gdb = GdbStub::new(conn);
        let _ = gdb.run_blocking::<WasmGdbEventLoop>(&mut target);

        // Put the machine back
        self.machine = Some(target.machine);

        // Retrieve the output from the thread-local buffer
        GDB_OUTPUT.with(|o| o.borrow().clone())
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
    pub fn set_adc_value(
        &mut self,
        peripheral_name: &str,
        value: u16,
    ) -> Result<(), JsValue> {
        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(peripheral_name)
            .ok_or_else(|| {
                JsValue::from_str(&format!("ADC '{}' not found", peripheral_name))
            })?;
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

    /// Returns analog state for ADC and PWM board_io bindings.
    #[wasm_bindgen]
    pub fn get_board_io_analog_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            match binding.kind {
                BoardIoKind::AdcInput => {
                    if let Some(idx) =
                        machine.bus.find_peripheral_index_by_name(&binding.peripheral)
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
}

struct WasmGdbEventLoop;

impl gdbstub::stub::run_blocking::BlockingEventLoop for WasmGdbEventLoop {
    type Target = LabwiredTarget<CortexM>;
    type Connection = WasmGdbConn;
    type StopReason = BaseStopReason<(), u32>;

    fn wait_for_stop_reason(
        _target: &mut Self::Target,
        _conn: &mut Self::Connection,
    ) -> Result<
        gdbstub::stub::run_blocking::Event<Self::StopReason>,
        gdbstub::stub::run_blocking::WaitForStopReasonError<
            <Self::Target as gdbstub::target::Target>::Error,
            <Self::Connection as Connection>::Error,
        >,
    > {
        // Signal stopped — gdbstub will send reply packets via write()
        Ok(gdbstub::stub::run_blocking::Event::TargetStopped(
            BaseStopReason::Signal(gdbstub::common::Signal::SIGTRAP),
        ))
    }

    fn on_interrupt(
        _target: &mut Self::Target,
    ) -> Result<Option<Self::StopReason>, <Self::Target as gdbstub::target::Target>::Error> {
        Ok(None)
    }
}
