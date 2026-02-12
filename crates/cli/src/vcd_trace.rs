use labwired_core::SimulationObserver;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use vcd::{Value, VarType};

pub struct VcdObserver<W: Write + Send + Sync> {
    writer: Mutex<vcd::Writer<W>>,
    timestamp: AtomicU64,
    pc_id: vcd::IdCode,
    addr_id: vcd::IdCode,
    data_id: vcd::IdCode,
    we_id: vcd::IdCode,
}

impl<W: Write + Send + Sync> VcdObserver<W> {
    pub fn new(sink: W) -> Self {
        let mut writer = vcd::Writer::new(sink);
        writer.timescale(1, vcd::TimescaleUnit::NS).unwrap();
        writer.add_module("top").unwrap();
        let pc_id = writer.add_var(VarType::Wire, 32, "pc", None).unwrap();

        writer.add_module("bus").unwrap();
        let addr_id = writer.add_var(VarType::Wire, 32, "addr", None).unwrap();
        let data_id = writer.add_var(VarType::Wire, 8, "data", None).unwrap();
        let we_id = writer.add_var(VarType::Wire, 1, "we", None).unwrap();
        writer.upscope().unwrap(); // exit bus

        writer.upscope().unwrap(); // exit top
        writer.enddefinitions().unwrap();

        Self {
            writer: Mutex::new(writer),
            timestamp: AtomicU64::new(0),
            pc_id,
            addr_id,
            data_id,
            we_id,
        }
    }

    fn u64_to_vcd_vector(val: u64, bits: u32) -> Vec<Value> {
        let mut vec = Vec::with_capacity(bits as usize);
        for i in (0..bits).rev() {
            if (val >> i) & 1 == 1 {
                vec.push(Value::V1);
            } else {
                vec.push(Value::V0);
            }
        }
        vec
    }
}

// Manually implement Debug for VcdObserver
impl<W: Write + Send + Sync> std::fmt::Debug for VcdObserver<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VcdObserver")
            .field("timestamp", &self.timestamp)
            .finish()
    }
}

impl<W: Write + Send + Sync> SimulationObserver for VcdObserver<W> {
    fn on_step_start(&self, pc: u32, _opcode: u32) {
        let t = self.timestamp.fetch_add(1, Ordering::SeqCst);
        let mut writer = self.writer.lock().unwrap();
        writer.timestamp(t).unwrap();
        writer
            .change_vector(self.pc_id, Self::u64_to_vcd_vector(pc as u64, 32))
            .unwrap();
        // Reset we signal on each step
        writer.change_scalar(self.we_id, Value::V0).unwrap();
    }

    fn on_memory_write(&self, addr: u64, _old: u8, new: u8) {
        let mut writer = self.writer.lock().unwrap();
        writer
            .change_vector(self.addr_id, Self::u64_to_vcd_vector(addr, 32))
            .unwrap();
        writer
            .change_vector(self.data_id, Self::u64_to_vcd_vector(new as u64, 8))
            .unwrap();
        writer.change_scalar(self.we_id, Value::V1).unwrap();
    }
}
