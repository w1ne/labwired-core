use labwired_core::{SimulationObserver};
use std::fs::File;
use std::io::BufWriter;
use std::sync::Mutex;
use vcd::{IdCode, TimescaleUnit, Value, Writer};

pub struct VcdObserver {
    state: Mutex<VcdState>,
    // Signal IDs
    ids: VcdIds,
    // Bit widths for conversion
    widths: VcdWidths,
}

struct VcdIds {
    pc: IdCode,
    mem_addr: IdCode,
    mem_data: IdCode,
    mem_we: IdCode,
}

struct VcdWidths {
    pc: u32,
    mem_addr: u32,
    mem_data: u32,
}

struct VcdState {
    writer: Writer<BufWriter<File>>,
    current_time: u64,
}

impl VcdObserver {
    pub fn new(path: std::path::PathBuf) -> anyhow::Result<Self> {
        let file = File::create(path)?;
        let buf = BufWriter::new(file);
        let mut writer = Writer::new(buf);

        // Header
        writer.timescale(1, TimescaleUnit::US)?;
        writer.add_module("top")?;

        let pc = writer.add_wire(32, "pc")?;

        writer.add_module("bus")?;
        let mem_addr = writer.add_wire(32, "addr")?;
        let mem_data = writer.add_wire(8, "data")?;
        let mem_we = writer.add_wire(1, "we")?;
        writer.upscope()?; // bus

        writer.upscope()?; // top
        writer.enddefinitions()?;

        // Initial values
        writer.timestamp(0)?;
        writer.change_vector(pc, u64_to_vec(0, 32))?;
        writer.change_vector(mem_addr, u64_to_vec(0, 32))?;
        writer.change_vector(mem_data, u64_to_vec(0, 8))?;
        writer.change_scalar(mem_we, Value::V0)?;

        Ok(Self {
            state: Mutex::new(VcdState {
                writer,
                current_time: 0,
            }),
            ids: VcdIds {
                pc,
                mem_addr,
                mem_data,
                mem_we,
            },
            widths: VcdWidths {
                pc: 32,
                mem_addr: 32,
                mem_data: 8,
            },
        })
    }
}

// Helper to convert u64 to Vec<Value> (MSB first)
fn u64_to_vec(val: u64, width: u32) -> Vec<Value> {
    let mut bits = Vec::with_capacity(width as usize);
    for i in (0..width).rev() {
        let bit = (val >> i) & 1;
        bits.push(if bit == 1 { Value::V1 } else { Value::V0 });
    }
    bits
}

impl core::fmt::Debug for VcdObserver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "VcdObserver")
    }
}

impl SimulationObserver for VcdObserver {
    fn on_step_start(&self, pc: u32, _opcode: u32) {
        if let Ok(mut state) = self.state.lock() {
            let width = self.widths.pc;
            let val = u64_to_vec(pc as u64, width);
            let _ = state.writer.change_vector(self.ids.pc, val);

            let _ = state.writer.change_scalar(self.ids.mem_we, Value::V0);
        }
    }

    fn on_step_end(&self, cycles: u32) {
        if let Ok(mut state) = self.state.lock() {
            state.current_time += cycles as u64;
            let time = state.current_time;
            let _ = state.writer.timestamp(time);
        }
    }

    fn on_memory_write(&self, addr: u64, _old: u8, new: u8) {
        if let Ok(mut state) = self.state.lock() {
             let addr_width = self.widths.mem_addr;
             let data_width = self.widths.mem_data;

             let addr_vec = u64_to_vec(addr, addr_width);
             let data_vec = u64_to_vec(new as u64, data_width);

             let _ = state.writer.change_vector(self.ids.mem_addr, addr_vec);
             let _ = state.writer.change_vector(self.ids.mem_data, data_vec);
             let _ = state.writer.change_scalar(self.ids.mem_we, Value::V1);
        }
    }
}
