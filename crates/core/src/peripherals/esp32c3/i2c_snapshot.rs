//! Complete local I2C controller resume, including a partially clocked byte.
use super::*;
use crate::peripherals::esp_i2c_core::EspI2cCoreSnapshot;

#[derive(serde::Serialize, serde::Deserialize)]
struct I2cSnapshot {
    tag: u32,
    version: u8,
    cpu_clk_hz: u64,
    intr_source_id: u32,
    on_net: bool,
    pending_cycles: u64,
    core: EspI2cCoreSnapshot,
    ctr: u32,
    sr: u32,
    slave_addr: u32,
    int_raw: u32,
    int_ena: u32,
    fifo_conf: u32,
    cmds: [u32; NUM_CMDS],
    tx_fifo: std::collections::VecDeque<u8>,
    tx_pop_count: usize,
    active_slave: Option<usize>,
    expects_addr: bool,
    engine: BitEngine,
    arm_seq: u32,
    reg_scl_low_period: u32,
    reg_to: u32,
    reg_sda_hold: u32,
    reg_sda_sample: u32,
    reg_scl_high_period: u32,
    reg_scl_start_hold: u32,
    reg_scl_rstart_setup: u32,
    reg_scl_stop_hold: u32,
    reg_scl_stop_setup: u32,
    reg_filter_cfg: u32,
    reg_clk_conf: u32,
    reg_scl_st_time_out: u32,
    reg_scl_main_st_time_out: u32,
    reg_scl_sp_conf: u32,
    reg_scl_stretch_conf: u32,
    reg_date: u32,
}

impl Esp32c3I2c {
    pub(super) fn settle_runtime_pending_cycles(&mut self) {
        let pending = std::mem::take(&mut self.resume_pending_cycles);
        if pending != 0 {
            self.advance_engine(pending);
        }
    }

    pub(super) fn capture_runtime_state(&self) -> Vec<u8> {
        let state = I2cSnapshot {
            tag: 0x4c57_4332,
            version: 1,
            cpu_clk_hz: self.cpu_clk_hz,
            intr_source_id: self.intr_source_id,
            on_net: self.net_mode(),
            pending_cycles: self.resume_pending_cycles.saturating_add(
                self.clock
                    .as_ref()
                    .map_or(0, |c| c.now().saturating_sub(self.last_synced)),
            ),
            core: self.core.runtime_snapshot(),
            ctr: self.ctr,
            sr: self.sr,
            slave_addr: self.slave_addr,
            int_raw: self.int_raw,
            int_ena: self.int_ena,
            fifo_conf: self.fifo_conf,
            cmds: self.cmds,
            tx_fifo: self.tx_fifo.clone(),
            tx_pop_count: self.tx_pop_count,
            active_slave: self.active_slave,
            expects_addr: self.expects_addr,
            engine: self.engine.clone(),
            arm_seq: self.arm_seq,
            reg_scl_low_period: self.reg_scl_low_period,
            reg_to: self.reg_to,
            reg_sda_hold: self.reg_sda_hold,
            reg_sda_sample: self.reg_sda_sample,
            reg_scl_high_period: self.reg_scl_high_period,
            reg_scl_start_hold: self.reg_scl_start_hold,
            reg_scl_rstart_setup: self.reg_scl_rstart_setup,
            reg_scl_stop_hold: self.reg_scl_stop_hold,
            reg_scl_stop_setup: self.reg_scl_stop_setup,
            reg_filter_cfg: self.reg_filter_cfg,
            reg_clk_conf: self.reg_clk_conf,
            reg_scl_st_time_out: self.reg_scl_st_time_out,
            reg_scl_main_st_time_out: self.reg_scl_main_st_time_out,
            reg_scl_sp_conf: self.reg_scl_sp_conf,
            reg_scl_stretch_conf: self.reg_scl_stretch_conf,
            reg_date: self.reg_date,
        };
        bincode::serialize(&state).expect("serialize C3 I2C runtime state")
    }

    pub(super) fn restore_runtime_state(&mut self, bytes: &[u8]) -> SimResult<()> {
        // Existing shipped boot captures precede I2C/display initialization.
        if bytes.is_empty() {
            return Ok(());
        }
        let state: I2cSnapshot = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("I2C snapshot decode: {e}"))
        })?;
        if state.tag != 0x4c57_4332
            || state.version != 1
            || state.cpu_clk_hz != self.cpu_clk_hz
            || state.intr_source_id != self.intr_source_id
            || state.on_net
            || self.net_mode()
            || state.tx_fifo.len() > FIFO_CAPACITY
            || state
                .active_slave
                .is_some_and(|i| i >= self.core.slave_count())
            || state.engine.cmd_idx > NUM_CMDS
            // The completed ACK advances 8 -> 9 before STOP/END/START or
            // idle. Only a segment that still clocks a bit must stay <= 8.
            || state.engine.bit_idx > 9
            || (state.engine.bit_idx > 8
                && matches!(
                    state.engine.state,
                    EngineState::BitLowHold | EngineState::BitLowDrive | EngineState::BitHigh
                ))
            || state.engine.timing.num == 0
            || state.engine.timing.den == 0
        {
            return Err(crate::SimulationError::NotImplemented(
                "I2C snapshot has incompatible clock, topology or wire state".into(),
            ));
        }
        self.core.restore_runtime_snapshot(state.core)?;
        self.ctr = state.ctr;
        self.sr = state.sr;
        self.slave_addr = state.slave_addr;
        self.int_raw = state.int_raw;
        self.int_ena = state.int_ena;
        self.fifo_conf = state.fifo_conf;
        self.cmds = state.cmds;
        self.tx_fifo = state.tx_fifo;
        self.tx_pop_count = state.tx_pop_count;
        self.active_slave = state.active_slave;
        self.expects_addr = state.expects_addr;
        self.engine = state.engine;
        self.arm_seq = state.arm_seq;
        self.reg_scl_low_period = state.reg_scl_low_period;
        self.reg_to = state.reg_to;
        self.reg_sda_hold = state.reg_sda_hold;
        self.reg_sda_sample = state.reg_sda_sample;
        self.reg_scl_high_period = state.reg_scl_high_period;
        self.reg_scl_start_hold = state.reg_scl_start_hold;
        self.reg_scl_rstart_setup = state.reg_scl_rstart_setup;
        self.reg_scl_stop_hold = state.reg_scl_stop_hold;
        self.reg_scl_stop_setup = state.reg_scl_stop_setup;
        self.reg_filter_cfg = state.reg_filter_cfg;
        self.reg_clk_conf = state.reg_clk_conf;
        self.reg_scl_st_time_out = state.reg_scl_st_time_out;
        self.reg_scl_main_st_time_out = state.reg_scl_main_st_time_out;
        self.reg_scl_sp_conf = state.reg_scl_sp_conf;
        self.reg_scl_stretch_conf = state.reg_scl_stretch_conf;
        self.reg_date = state.reg_date;
        // Do not clock bytes during restore: another peripheral, such as
        // GPIO's route matrix, may not have been restored yet. Preserve the
        // lazy source-clock debt until the first normal execution boundary.
        self.resume_pending_cycles = state.pending_cycles;
        self.last_synced = self.clock.as_ref().map_or(0, CycleClock::now);
        self.scheduled = false;
        self.net_irq = Default::default();
        if let Some(lines) = &self.lines {
            lines.set(self.engine.scl, self.engine.sda);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::components::{
        ssd1306,
        supply_domain::{GatedI2cDevice, SupplyGate},
    };

    fn fixture() -> Esp32c3I2c {
        let mut dev = Esp32c3I2c::new();
        let trace = crate::bus::bus_trace::BusTrace::default();
        dev.push_slave(crate::bus::bus_trace::wrap_i2c(
            "i2c0",
            &trace,
            Box::new(GatedI2cDevice::new(
                Box::new(ssd1306(0x3c)),
                SupplyGate::legacy(true),
            )),
        ));
        dev
    }

    #[test]
    fn i2c_runtime_snapshot_reanchors_nonzero_lazy_cycles_without_moving_wire_edges() {
        let mut original = fixture();
        let route = C3I2cPadRoute { sda: 4, scl: 5 };
        original.slave_routes[0] = Some(route);
        let original_matrix = Arc::new(Mutex::new(C3I2cMatrixRouteState::default()));
        original_matrix
            .lock()
            .unwrap()
            .set(1 << 4, 1 << 5, Some(4), Some(5));
        original.set_matrix_route_state(original_matrix);
        let original_clock = CycleClock::default();
        original.attach_cycle_clock(original_clock.clone());
        original.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        original.write_u32(REG_SCL_LOW_PERIOD, 4).unwrap();
        original.write_u32(REG_CMD0, OP_RSTART << 11).unwrap();
        original
            .write_u32(REG_CMD0 + 4, (OP_WRITE << 11) | 4)
            .unwrap();
        original.write_u32(REG_CMD0 + 8, OP_STOP << 11).unwrap();
        for byte in [0x78, 0x40, 0xaa, 0x55] {
            original.write_u32(REG_DATA, byte).unwrap();
        }
        original.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        original_clock.publish(71);
        original.sync_to(71);
        original_clock.publish(300);
        let snapshot = original.runtime_snapshot();
        assert_eq!(
            bincode::deserialize::<I2cSnapshot>(&snapshot)
                .unwrap()
                .pending_cycles,
            229
        );

        let mut restored = fixture();
        restored.slave_routes[0] = Some(route);
        let restored_matrix = Arc::new(Mutex::new(C3I2cMatrixRouteState::default()));
        restored.set_matrix_route_state(restored_matrix.clone());
        let restored_clock = CycleClock::default();
        restored.attach_cycle_clock(restored_clock.clone());
        restored.restore_runtime_snapshot(&snapshot).unwrap();
        assert_eq!(restored.resume_pending_cycles, 229);
        assert_eq!(
            restored.active_slave, None,
            "restore must not resolve through cold GPIO routing"
        );
        // GPIO may restore AFTER I2C. Only then may the elapsed address phase
        // resolve the slave during the first normal scheduler bootstrap.
        restored_matrix
            .lock()
            .unwrap()
            .set(1 << 4, 1 << 5, Some(4), Some(5));
        restored.sync_to(0);
        original.sync_to(300);
        assert_eq!(original.active_slave, Some(0));
        assert_eq!(
            bincode::serialize(&restored.engine).unwrap(),
            bincode::serialize(&original.engine).unwrap()
        );
        let initial_lines = (original.engine.scl, original.engine.sda);
        let mut first_edge = None;
        let mut first_irq = None;
        for elapsed in 1..=2000 {
            original_clock.publish(300 + elapsed);
            restored_clock.publish(elapsed);
            original.sync_to(300 + elapsed);
            restored.sync_to(elapsed);
            let lines = (original.engine.scl, original.engine.sda);
            assert_eq!(lines, (restored.engine.scl, restored.engine.sda));
            assert_eq!(
                original.int_raw & original.int_ena,
                restored.int_raw & restored.int_ena
            );
            if lines != initial_lines && first_edge.is_none() {
                first_edge = Some(elapsed);
            }
            if original.int_raw & original.int_ena != 0 && first_irq.is_none() {
                first_irq = Some(elapsed);
            }
        }
        assert!(first_edge.is_some());
        assert!(first_irq.is_some());
        assert_eq!(original.runtime_snapshot(), restored.runtime_snapshot());
    }

    #[test]
    fn i2c_runtime_snapshot_refuses_unimplemented_wrapper_state() {
        struct StatefulWrapper(Box<dyn I2cDevice>);
        impl I2cDevice for StatefulWrapper {
            fn address(&self) -> u8 {
                self.0.address()
            }
            fn read(&mut self) -> u8 {
                self.0.read()
            }
            fn write(&mut self, value: u8) {
                self.0.write(value);
            }
            fn wrapped_device(&self) -> Option<&dyn I2cDevice> {
                Some(self.0.as_ref())
            }
            fn wrapped_device_mut(&mut self) -> Option<&mut dyn I2cDevice> {
                Some(self.0.as_mut())
            }
        }
        let make = || {
            let mut dev = Esp32c3I2c::new();
            dev.push_slave(Box::new(StatefulWrapper(Box::new(ssd1306(0x3c)))));
            dev
        };
        let source = make();
        let mut target = make();
        assert!(target
            .restore_runtime_snapshot(&source.runtime_snapshot())
            .is_err());
    }

    #[test]
    fn i2c_runtime_snapshot_rejects_out_of_range_active_bit_indices() {
        let source = fixture();
        let mut target = fixture();
        let before = target.runtime_snapshot();
        for (phase, bit_idx) in [
            (EngineState::BitLowHold, 9),
            (EngineState::BitLowDrive, 9),
            (EngineState::BitHigh, 9),
            (EngineState::Idle, 10),
        ] {
            let mut state: I2cSnapshot = bincode::deserialize(&source.runtime_snapshot()).unwrap();
            state.engine.state = phase;
            state.engine.bit_idx = bit_idx;
            assert!(target
                .restore_runtime_snapshot(&bincode::serialize(&state).unwrap())
                .is_err());
            assert_eq!(target.runtime_snapshot(), before);
        }
    }

    #[test]
    fn i2c_runtime_snapshot_resumes_a_partial_wire_transfer_and_retains_the_oled() {
        let mut original = fixture();
        original.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        original.write_u32(REG_SCL_LOW_PERIOD, 4).unwrap();
        original.write_u32(REG_CMD0, OP_RSTART << 11).unwrap();
        original
            .write_u32(REG_CMD0 + 4, (OP_WRITE << 11) | 4)
            .unwrap();
        original.write_u32(REG_CMD0 + 8, OP_STOP << 11).unwrap();
        for byte in [0x78, 0x40, 0xaa, 0x55] {
            original.write_u32(REG_DATA, byte).unwrap();
        }
        original.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        original.tick_elapsed(71);
        assert!(original.engine_active());
        let mut restored = fixture();
        restored
            .restore_runtime_snapshot(&original.runtime_snapshot())
            .unwrap();
        for _ in 0..2000 {
            original.tick_elapsed(1);
            restored.tick_elapsed(1);
            assert_eq!(original.engine.scl, restored.engine.scl);
            assert_eq!(original.engine.sda, restored.engine.sda);
            assert_eq!(original.int_raw, restored.int_raw);
        }
        assert!(!restored.engine_active());
        assert_eq!(restored.engine.state, EngineState::Idle);
        assert_eq!(
            restored.engine.bit_idx, 9,
            "the completed ACK index is retained"
        );
        let opts = crate::inspect::InspectOpts::default();
        let expected = original.inspect(0, "i2c0", &opts).artifacts;
        let actual = restored.inspect(0, "i2c0", &opts).artifacts;
        assert_eq!(expected[0].meta, actual[0].meta);
        assert_eq!(actual[0].meta["lit_pixels"], 8);
        let mut fresh = fixture();
        fresh
            .restore_runtime_snapshot(&restored.runtime_snapshot())
            .unwrap();
        assert_eq!(
            fresh.inspect(0, "i2c0", &opts).artifacts[0].meta,
            actual[0].meta
        );
        assert_eq!(fresh.engine.bit_idx, 9);
        assert_eq!(fresh.read_u32(REG_SCL_LOW_PERIOD).unwrap(), 4);
        assert_eq!(fresh.read_u32(REG_INT_ENA).unwrap(), INT_TRANS_COMPLETE);
    }
}
