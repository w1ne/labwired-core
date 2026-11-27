//! GPIO register and matrix state; fresh topology keeps its shared wire cells.
use super::*;

#[derive(serde::Serialize, serde::Deserialize)]
struct GpioSnapshot {
    tag: u32,
    version: u8,
    pin_count: u8,
    intr_source: u32,
    c5_register_map: bool,
    c61_register_map: bool,
    registers: [u32; 8],
    pin_cfg: Vec<u32>,
    out_sel: Vec<u32>,
    in_sel: Vec<u32>,
    on_net: bool,
}

impl Esp32c3Gpio {
    pub(super) fn capture_runtime_state(&self) -> Vec<u8> {
        bincode::serialize(&GpioSnapshot {
            tag: 0x4c57_4347,
            version: 1,
            pin_count: self.pin_count,
            intr_source: self.intr_source,
            c5_register_map: self.c5_register_map,
            c61_register_map: self.c61_register_map,
            registers: [
                self.bt_select,
                self.out,
                self.sdio_select,
                self.enable,
                self.strap,
                self.external_levels,
                self.external_drive_mask,
                self.status,
            ],
            pin_cfg: self.pin_cfg.to_vec(),
            out_sel: self.out_sel.to_vec(),
            in_sel: self.in_sel.to_vec(),
            on_net: self.net_pads != 0,
        })
        .expect("serialize C3 GPIO runtime snapshot")
    }

    pub(super) fn restore_runtime_state(&mut self, bytes: &[u8]) -> SimResult<()> {
        // Legacy shipped boot captures precede GPIO initialization.
        if bytes.is_empty() {
            return Ok(());
        }
        let state: GpioSnapshot = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("GPIO snapshot decode: {e}"))
        })?;
        if state.tag != 0x4c57_4347
            || state.version != 1
            || state.pin_count != self.pin_count
            || state.intr_source != self.intr_source
            || state.c5_register_map != self.c5_register_map
            || state.c61_register_map != self.c61_register_map
            || state.pin_cfg.len() != self.pin_cfg.len()
            || state.out_sel.len() != self.out_sel.len()
            || state.in_sel.len() != self.in_sel.len()
            || state.on_net
            || self.net_pads != 0
        {
            return Err(crate::SimulationError::NotImplemented(
                "GPIO snapshot has incompatible geometry or world wiring".into(),
            ));
        }
        [
            self.bt_select,
            self.out,
            self.sdio_select,
            self.enable,
            self.strap,
            self.external_levels,
            self.external_drive_mask,
            self.status,
        ] = state.registers;
        self.pin_cfg.copy_from_slice(&state.pin_cfg);
        self.out_sel.copy_from_slice(&state.out_sel);
        self.in_sel.copy_from_slice(&state.in_sel);
        // Keep the destination's wire handles and time origin. Rebuild the
        // shared matrix cell so the I2C controller sees the restored registers.
        self.after_matrix_write();
        self.irq_watch = Default::default();
        self.wire_edges.clear();
        for pin in 0..self.pin_count {
            self.notify_pad_source(pin);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpio_runtime_snapshot_restores_registers_and_shared_i2c_route() {
        let mut original = Esp32c3Gpio::new();
        original
            .write_u32(ENABLE_W1TS, (1 << 4) | (1 << 5))
            .unwrap();
        original
            .write_u32(FUNC_OUT_SEL + 4 * 4, original.layout.i2c_sda)
            .unwrap();
        original
            .write_u32(FUNC_OUT_SEL + 4 * 5, original.layout.i2c_scl)
            .unwrap();
        original
            .write_u32(
                FUNC_IN_SEL + 4 * u64::from(original.layout.i2c_sda),
                (1 << 6) | 4,
            )
            .unwrap();
        original
            .write_u32(
                FUNC_IN_SEL + 4 * u64::from(original.layout.i2c_scl),
                (1 << 6) | 5,
            )
            .unwrap();
        original.write_u32(OUT, 0x10).unwrap();
        let bytes = original.runtime_snapshot();
        let mut restored = Esp32c3Gpio::new();
        let route = restored.i2c_matrix_route_state();
        restored.restore_runtime_snapshot(&bytes).unwrap();
        assert_eq!(restored.runtime_snapshot(), bytes);
        assert_eq!(restored.enable, original.enable);
        assert_eq!(restored.out, original.out);
        assert_eq!(
            format!("{:?}", route.lock().unwrap()),
            format!("{:?}", original.i2c_matrix_route.lock().unwrap())
        );
    }
}
