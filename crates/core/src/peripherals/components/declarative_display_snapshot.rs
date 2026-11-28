//! Full I2C panel state. Unlike a picture export, a live resume preserves
//! the address cursor and a partially received command/data stream.
use super::*;

#[derive(serde::Serialize, serde::Deserialize)]
struct I2cDisplaySnapshot {
    tag: u32,
    version: u8,
    spec_json: Vec<u8>,
    address: u8,
    params: Vec<u8>,
    display_on: bool,
    awake: bool,
    inverted: bool,
    mode: DisplayAddressingMode,
    vars: BTreeMap<String, u32>,
    col: u16,
    row: u16,
    page: u16,
    col_start: u16,
    col_end: u16,
    row_start: u16,
    row_end: u16,
    page_start: u16,
    page_end: u16,
    ram: Vec<u8>,
    screen: Vec<u8>,
    refresh_generation: u32,
    generation_history: VecDeque<u64>,
    frames_seen: VecDeque<LoggedFrame>,
    ram_stream_wrote: bool,
    ram_bytes_since_note: usize,
    last_logged_generation: Option<u64>,
    framing: Framing,
    pending_cmd: u8,
    pending_idx: Option<u16>,
    param_have: u8,
    param_want: u8,
    unit: [u8; 4],
    unit_have: usize,
    plane: usize,
    ram_remaining: Option<u32>,
    control: Option<u8>,
    nibble_e: bool,
    nibble_pending: Option<(u8, bool)>,
    elapsed_us: u64,
}

impl GenericDisplay {
    pub(super) fn capture_i2c_runtime_state(&self) -> Option<Vec<u8>> {
        // Serial-colour decoder state is not modeled by this I2C transport.
        if self.serial_color.is_some() {
            return None;
        }
        let state = I2cDisplaySnapshot {
            tag: 0x4c57_4944,
            version: 1,
            spec_json: serde_json::to_vec(&self.spec).expect("serialize display descriptor"),
            address: self.address,
            params: self.params.to_vec(),
            display_on: self.display_on,
            awake: self.awake,
            inverted: self.inverted,
            mode: self.mode,
            vars: self.vars.clone(),
            col: self.col,
            row: self.row,
            page: self.page,
            col_start: self.col_start,
            col_end: self.col_end,
            row_start: self.row_start,
            row_end: self.row_end,
            page_start: self.page_start,
            page_end: self.page_end,
            ram: self.ram.clone(),
            screen: self.screen.clone(),
            refresh_generation: self.refresh_generation,
            generation_history: self.generation_history.clone(),
            frames_seen: self.frames_seen.clone(),
            ram_stream_wrote: self.ram_stream_wrote,
            ram_bytes_since_note: self.ram_bytes_since_note,
            last_logged_generation: self.last_logged_generation,
            framing: self.framing,
            pending_cmd: self.pending_cmd,
            pending_idx: self.pending_idx,
            param_have: self.param_have,
            param_want: self.param_want,
            unit: self.unit,
            unit_have: self.unit_have,
            plane: self.plane,
            ram_remaining: self.ram_remaining,
            control: self.control,
            nibble_e: self.nibble_e,
            nibble_pending: self.nibble_pending,
            elapsed_us: self.elapsed_us,
        };
        Some(bincode::serialize(&state).expect("serialize I2C display state"))
    }

    pub(super) fn restore_i2c_runtime_state(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        let state: I2cDisplaySnapshot = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("I2C display snapshot decode: {e}"))
        })?;
        if state.tag != 0x4c57_4944
            || state.version != 1
            || self.serial_color.is_some()
            || state.address != self.address
            || state.spec_json
                != serde_json::to_vec(&self.spec).expect("serialize display descriptor")
            || state.ram.len() != self.ram.len()
            || state.screen.len() != self.screen.len()
            || state.params.len() != self.params.len()
            || usize::from(state.param_have) > state.params.len()
            || usize::from(state.param_want) > state.params.len()
            || state.unit_have > self.unit.len()
            || state.plane >= self.plane_names.len().max(1)
            || state
                .pending_idx
                .is_some_and(|idx| usize::from(idx) >= self.spec.commands.len())
        {
            return Err(crate::SimulationError::NotImplemented(
                "I2C display snapshot has incompatible descriptor or protocol state".into(),
            ));
        }
        self.params.copy_from_slice(&state.params);
        self.display_on = state.display_on;
        self.awake = state.awake;
        self.inverted = state.inverted;
        self.mode = state.mode;
        self.vars = state.vars;
        self.col = state.col;
        self.row = state.row;
        self.page = state.page;
        self.col_start = state.col_start;
        self.col_end = state.col_end;
        self.row_start = state.row_start;
        self.row_end = state.row_end;
        self.page_start = state.page_start;
        self.page_end = state.page_end;
        self.ram = state.ram;
        self.screen = state.screen;
        self.refresh_generation = state.refresh_generation;
        self.generation_history = state.generation_history;
        self.frames_seen = state.frames_seen;
        self.ram_stream_wrote = state.ram_stream_wrote;
        self.ram_bytes_since_note = state.ram_bytes_since_note;
        self.last_logged_generation = state.last_logged_generation;
        self.framing = state.framing;
        self.pending_cmd = state.pending_cmd;
        self.pending_idx = state.pending_idx;
        self.param_have = state.param_have;
        self.param_want = state.param_want;
        self.unit = state.unit;
        self.unit_have = state.unit_have;
        self.plane = state.plane;
        self.ram_remaining = state.ram_remaining;
        self.control = state.control;
        self.nibble_e = state.nibble_e;
        self.nibble_pending = state.nibble_pending;
        self.elapsed_us = state.elapsed_us;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::components::ssd1306;

    #[test]
    fn i2c_display_runtime_snapshot_keeps_pixels_flags_and_partial_protocol() {
        let mut original = ssd1306(0x3c);
        I2cDevice::start(&mut original);
        I2cDevice::write(&mut original, 0x00);
        I2cDevice::write(&mut original, 0xaf); // display on
        I2cDevice::write(&mut original, 0x21); // column window; arguments pending
        I2cDevice::write(&mut original, 4);
        let mut restored = ssd1306(0x3c);
        I2cDevice::restore_runtime_snapshot(
            &mut restored,
            &I2cDevice::runtime_snapshot(&original).unwrap(),
        )
        .unwrap();
        for panel in [&mut original, &mut restored] {
            I2cDevice::write(panel, 7); // finish the interrupted command
            I2cDevice::stop(panel);
            I2cDevice::start(panel);
            I2cDevice::write(panel, 0x40);
            I2cDevice::write(panel, 0xaa);
            I2cDevice::stop(panel);
        }
        assert!(restored.display_on());
        assert_eq!(restored.lit_pixels(), 4);
        assert_eq!(restored.ram[4], 0xaa);
        assert_eq!(
            I2cDevice::runtime_snapshot(&original),
            I2cDevice::runtime_snapshot(&restored)
        );
        let mut fresh = ssd1306(0x3c);
        I2cDevice::restore_runtime_snapshot(
            &mut fresh,
            &I2cDevice::runtime_snapshot(&restored).unwrap(),
        )
        .unwrap();
        assert_eq!(fresh.ram, restored.ram);
        assert!(fresh.display_on());
    }
}
