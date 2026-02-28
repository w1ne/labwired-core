use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::decoder::arm::{decode_thumb_16, decode_thumb_32};
use labwired_core::memory::LinearMemory;
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
    static GDB_OUTPUT: RefCell<Vec<u8>> = RefCell::new(Vec::new());
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
}

#[wasm_bindgen]
impl WasmSimulator {
    #[wasm_bindgen(constructor)]
    pub fn new(firmware: &[u8]) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::new();
        bus.flash = LinearMemory::new(128 * 1024, 0x0800_0000);
        bus.ram = LinearMemory::new(20 * 1024, 0x2000_0000);
        bus.refresh_peripheral_index();

        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        machine
            .load_firmware(&program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;

        Ok(WasmSimulator {
            machine: Some(machine),
        })
    }

    fn machine(&mut self) -> &mut Machine<CortexM> {
        self.machine.as_mut().unwrap()
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

    #[wasm_bindgen]
    pub fn get_led_state(&mut self) -> bool {
        let odr = self.machine().bus.read_u32(0x4001080C).unwrap_or(0);
        (odr >> 5) & 1 == 1
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
