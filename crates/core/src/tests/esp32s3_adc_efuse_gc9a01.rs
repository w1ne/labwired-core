// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Integration regression tests for ESP32-S3:
//! 1. Shared physical ADC1 pad bank between `sar_adc_s3` (0x6004_0000) and
//!    `sens_s3` (0x6000_8800) so `analogRead()` / `adc_oneshot` observes
//!    stimuli seeded on `sar_adc_s3` (and vice versa).
//! 2. ESP32-S3 eFuse factory V1 ADC calibration block (`0x6000_7044..=0x6000_7078`).
//! 3. ESP32-S3 SPI2 + GDMA + GC9A01 live D/C & CS pin gating.

#[cfg(test)]
mod esp32s3_adc_efuse_gc9a01_tests {
    use crate::bus::external_devices::{attach_external_device_universal, UniversalResolution};
    use crate::bus::SystemBus;
    use crate::inspect::InspectOpts;
    use crate::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
    use crate::Bus;

    const SENS_BASE: u64 = 0x6000_8800;
    const SAR_MEAS1_CTRL2: u64 = SENS_BASE + 0x00C;
    const MEAS_DONE_BIT: u32 = 1 << 16;
    const MEAS_START_BIT: u32 = 1 << 17;
    const MEAS_START_FORCE: u32 = 1 << 18;
    const MEAS_EN_PAD_SHIFT: u32 = 19;

    const APB_SARADC_BASE: u64 = 0x6004_0000;
    const APB_SARADC_DATA1: u64 = APB_SARADC_BASE + 0x40;
    const APB_SARADC_ONETIME: u64 = APB_SARADC_BASE + 0x80;

    const EFUSE_BASE: u64 = 0x6000_7000;

    const GPIO_BASE: u64 = 0x6000_4000;
    const GPIO_OUT_W1TS: u64 = GPIO_BASE + 0x08;
    const GPIO_OUT_W1TC: u64 = GPIO_BASE + 0x0C;
    const GPIO_ENABLE_W1TS: u64 = GPIO_BASE + 0x24;

    const SPI2_BASE: u64 = 0x6002_4000;
    const SPI_CMD: u64 = SPI2_BASE;
    const SPI_USER: u64 = SPI2_BASE + 0x10;
    const SPI_MS_DLEN: u64 = SPI2_BASE + 0x1C;
    const SPI_DMA_CONF: u64 = SPI2_BASE + 0x30;
    const SPI_W0: u64 = SPI2_BASE + 0x98;

    const GDMA_BASE: u64 = 0x6003_F000;
    const GDMA_OUT_LINK_CH0: u64 = GDMA_BASE + 0x080;
    const GDMA_OUT_PERI_SEL_CH0: u64 = GDMA_BASE + 0x0A8;

    const DRAM_BASE: u64 = 0x3FC8_8000;

    fn make_esp32s3_bus() -> SystemBus {
        let mut bus = SystemBus::new();
        let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        bus
    }

    fn sens_oneshot_adc1(bus: &mut SystemBus, ch: u8) -> u16 {
        // Arm channel & clear START_SAR, then rising-edge START_SAR (exact
        // sequence used by Arduino `analogRead()` / ESP-IDF `adc_oneshot`).
        let pad_mask = (1u32 << ch) << MEAS_EN_PAD_SHIFT;
        bus.write_u32(SAR_MEAS1_CTRL2, MEAS_START_FORCE | pad_mask)
            .unwrap();
        bus.write_u32(
            SAR_MEAS1_CTRL2,
            MEAS_START_FORCE | MEAS_START_BIT | pad_mask,
        )
        .unwrap();
        let word = bus.read_u32(SAR_MEAS1_CTRL2).unwrap();
        assert_ne!(word & MEAS_DONE_BIT, 0, "SENS oneshot must assert DONE");
        (word & 0xFFFF) as u16
    }

    fn apb_saradc_onetime_adc1(bus: &mut SystemBus, ch: u8) -> u16 {
        let onetime = (1u32 << 31) | (1u32 << 29) | ((u32::from(ch) & 0xF) << 25);
        bus.write_u32(APB_SARADC_ONETIME, onetime).unwrap();
        (bus.read_u32(APB_SARADC_DATA1).unwrap() & 0x0FFF) as u16
    }

    #[test]
    fn adc1_seeding_shared_between_sar_adc_s3_and_sens_s3() {
        let mut bus = make_esp32s3_bus();

        // Baseline: unseeded SENS oneshot falls back to mid-scale 2048.
        assert_eq!(sens_oneshot_adc1(&mut bus, 3), 2048);

        // Seeding `sar_adc_s3` (the target `board-config/pin-mapping.ts` emits
        // for GPIO1..GPIO10) must immediately be observed by `sens_s3`.
        assert!(bus.seed_adc_channel("sar_adc_s3", 3, 2500));
        assert!(bus.seed_adc_channel("sar_adc_s3", 4, 825));

        let expected_ch3 = crate::peripherals::esp_xtensa_common::esp32s3_adc1_mv_to_raw(2500, 3);
        let expected_ch4 = crate::peripherals::esp_xtensa_common::esp32s3_adc1_mv_to_raw(825, 3);
        assert_eq!(sens_oneshot_adc1(&mut bus, 3), expected_ch3);
        assert_eq!(sens_oneshot_adc1(&mut bus, 4), expected_ch4);
        assert_eq!(apb_saradc_onetime_adc1(&mut bus, 3), expected_ch3);

        // Bidirectional: seeding `sens_s3` must also be observed by `sar_adc_s3`.
        assert!(bus.seed_adc_channel("sens_s3", 1, 1650));
        let expected_ch1 = crate::peripherals::esp_xtensa_common::esp32s3_adc1_mv_to_raw(1650, 3);
        assert_eq!(sens_oneshot_adc1(&mut bus, 1), expected_ch1);
        assert_eq!(apb_saradc_onetime_adc1(&mut bus, 1), expected_ch1);

        // Clearing via `sar_adc_s3` reverts `sens_s3` to mid-scale 2048.
        let sar_idx = bus
            .find_peripheral_index_by_name("sar_adc_s3")
            .expect("sar_adc_s3 present");
        assert!(bus.peripherals[sar_idx].dev.clear_adc_channel_input(3));
        assert_eq!(sens_oneshot_adc1(&mut bus, 3), 2048);

        // Snapshot + restore on `sar_adc_s3` must preserve the shared pad link
        // with `sens_s3`.
        assert!(bus.seed_adc_channel("sar_adc_s3", 2, 1100));
        let snap = bus.peripherals[sar_idx].dev.snapshot();
        assert!(bus.seed_adc_channel("sar_adc_s3", 2, 2900));
        assert_eq!(
            sens_oneshot_adc1(&mut bus, 2),
            crate::peripherals::esp_xtensa_common::esp32s3_adc1_mv_to_raw(2900, 3)
        );
        bus.peripherals[sar_idx].dev.restore(snap).unwrap();
        assert_eq!(
            sens_oneshot_adc1(&mut bus, 2),
            crate::peripherals::esp_xtensa_common::esp32s3_adc1_mv_to_raw(1100, 3)
        );
        // Further writes after restore must still propagate to `sens_s3`.
        assert!(bus.seed_adc_channel("sar_adc_s3", 2, 3300));
        assert_eq!(sens_oneshot_adc1(&mut bus, 2), 4095);
    }

    #[test]
    fn esp32s3_adc1_curve_fit_no_rolloff_near_3v() {
        let mut bus = make_esp32s3_bus();
        // Verify near-3 V sensor divider voltages (e.g. 2970 mV soil moisture
        // at 10% and 3100 mV LDR at 500 lx) round-trip through SENS oneshot +
        // ESP-IDF ATTEN3 curve fitting within <= 2 mV instead of rolling off.
        for mv in [0u16, 330, 660, 850, 1650, 2500, 2970, 3100, 3200] {
            assert!(bus.seed_adc_channel("sar_adc_s3", 4, mv));
            let sens_raw = sens_oneshot_adc1(&mut bus, 4);
            let apb_raw = apb_saradc_onetime_adc1(&mut bus, 4);
            assert_eq!(
                sens_raw, apb_raw,
                "SENS and APB_SARADC must agree at {mv} mV"
            );
            let recovered =
                crate::peripherals::esp_xtensa_common::esp32s3_atten3_curve_fit_mv(sens_raw);
            assert!(
                recovered.abs_diff(mv) <= 2,
                "ESP32-S3 ADC1 ATTEN3 roll-off at {mv} mV: raw={sens_raw} -> {recovered} mV"
            );
        }
    }

    #[test]
    fn esp32s3_bus_efuse_exposes_v1_adc_calibration() {
        let bus = make_esp32s3_bus();
        // EFUSE_RD_SYS_PART1_DATA4_REG (0x6000_706C): BLK_VERSION_MAJOR bits [1:0] == 1.
        let data4 = bus.read_u32(EFUSE_BASE + 0x06C).unwrap();
        assert_eq!(
            data4 & 0b11,
            1,
            "BLK_VERSION_MAJOR must be 1 for ADC calib V1"
        );
        // OCODE bits [20:13] == 100.
        assert_eq!((data4 >> 13) & 0xFF, 100);
        // EFUSE_RD_MAC_SPI_SYS_5_REG (0x6000_7058): ADC2_CAL_VOL_ATTEN3 bits [31:26] == 15.
        let mac5 = bus.read_u32(EFUSE_BASE + 0x058).unwrap();
        assert_eq!((mac5 >> 26) & 0x3F, 15);
    }

    fn spi2_pio_send(bus: &mut SystemBus, bytes: &[u8]) {
        assert!(!bytes.is_empty() && bytes.len() <= 4);
        let mut word = 0u32;
        for (i, &b) in bytes.iter().enumerate() {
            word |= u32::from(b) << (i * 8);
        }
        bus.write_u32(SPI_W0, word).unwrap();
        bus.write_u32(SPI_MS_DLEN, (bytes.len() as u32 * 8) - 1)
            .unwrap();
        bus.write_u32(SPI_USER, 1u32 << 27).unwrap(); // USR_MOSI
        bus.write_u32(SPI_CMD, 1u32 << 24).unwrap(); // USR
    }

    #[test]
    fn spi2_gdma_gc9a01_latches_live_dc_and_respects_cs() {
        let mut bus = make_esp32s3_bus();

        let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
            r#"
schema_version: "1.0"
name: "esp32s3-gc9a01-test"
chip: "esp32s3.yaml"
external_devices:
  - id: "tft"
    type: "gc9a01"
    connection: "spi2_s3"
    config:
      dc_pin: "GPIO13"
      cs_pin: "GPIO10"
"#,
        )
        .expect("parse test manifest");

        let res =
            attach_external_device_universal(&mut bus, &manifest, &manifest.external_devices[0])
                .expect("attach gc9a01");
        assert_eq!(res, UniversalResolution::Attached);

        // Configure GPIO10 (CS) and GPIO13 (DC) as outputs; CS=LOW, DC=LOW.
        bus.write_u32(GPIO_ENABLE_W1TS, (1 << 10) | (1 << 13))
            .unwrap();
        bus.write_u32(GPIO_OUT_W1TC, (1 << 10) | (1 << 13)).unwrap();

        // Wake & turn display on via PIO commands (CS=LOW, DC=LOW).
        spi2_pio_send(&mut bus, &[0x11]); // SLPOUT
        spi2_pio_send(&mut bus, &[0x29]); // DISPON
        spi2_pio_send(&mut bus, &[0x2A]); // CASET
        bus.write_u32(GPIO_OUT_W1TS, 1 << 13).unwrap(); // DC=HIGH (data)
        spi2_pio_send(&mut bus, &[0x00, 0x00, 0x00, 0x01]); // x: 0..1 (2 px)
        bus.write_u32(GPIO_OUT_W1TC, 1 << 13).unwrap(); // DC=LOW (cmd)
        spi2_pio_send(&mut bus, &[0x2B]); // RASET
        bus.write_u32(GPIO_OUT_W1TS, 1 << 13).unwrap(); // DC=HIGH (data)
        spi2_pio_send(&mut bus, &[0x00, 0x00, 0x00, 0x01]); // y: 0..1 (2 px)
        bus.write_u32(GPIO_OUT_W1TC, 1 << 13).unwrap(); // DC=LOW (cmd)
        spi2_pio_send(&mut bus, &[0x2C]); // RAMWR

        // Arm SPI2 DMA transfer WHILE DC is still LOW, then switch DC=HIGH
        // before kicking GDMA — `pump_spi` must sample live DC=HIGH so the
        // pixel payload is accepted as RAMWR data rather than misinterpreted
        // as commands.
        let desc_addr = DRAM_BASE + 0x1000;
        let buf_addr = DRAM_BASE + 0x1100;
        let pixels: [u8; 8] = [0xF8, 0x00, 0x07, 0xE0, 0x00, 0x1F, 0xFF, 0xFF];
        for (i, &b) in pixels.iter().enumerate() {
            bus.write_u8(buf_addr + i as u64, b).unwrap();
        }
        // GDMA lldesc: owner=1(bit31), eof=1(bit30), len=8(bits23:12), size=8(bits11:0)
        let dw0 = (1u32 << 31) | (1u32 << 30) | (8u32 << 12) | 8u32;
        bus.write_u32(desc_addr, dw0).unwrap();
        bus.write_u32(desc_addr + 4, buf_addr as u32).unwrap();
        bus.write_u32(desc_addr + 8, 0).unwrap();

        bus.write_u32(SPI_DMA_CONF, 1u32 << 28).unwrap(); // DMA_TX_ENA
        bus.write_u32(SPI_MS_DLEN, (pixels.len() as u32 * 8) - 1)
            .unwrap();
        bus.write_u32(SPI_USER, 1u32 << 27).unwrap(); // USR_MOSI
        bus.write_u32(SPI_CMD, 1u32 << 24).unwrap(); // arm DMA pending with DC=LOW

        // Switch DC=HIGH AFTER SPI_CMD.USR was written, then start GDMA CH0 -> SPI2 (peri_sel=0).
        bus.write_u32(GPIO_OUT_W1TS, 1 << 13).unwrap();
        bus.write_u32(GDMA_OUT_PERI_SEL_CH0, 0).unwrap();
        bus.write_u32(
            GDMA_OUT_LINK_CH0,
            (1u32 << 21) | ((desc_addr as u32) & 0x000F_FFFF),
        )
        .unwrap();
        bus.tick_peripherals();

        let opts = InspectOpts {
            include_bytes: true,
            peripheral: None,
        };
        let arts = bus.display_artifacts_of_format(&["rgb565_be"], &opts);
        assert_eq!(
            arts.len(),
            1,
            "expected 1 gc9a01 rgb565_be display artifact"
        );
        let art = &arts[0];
        assert_eq!(art.meta["awake"], true);
        assert_eq!(art.meta["display_on"], true);
        // `painted_bytes` counts non-zero bytes written to the framebuffer:
        // [0xF8, 0x00, 0x07, 0xE0, 0x00, 0x1F, 0xFF, 0xFF] has 6 non-zero bytes.
        assert_eq!(
            art.meta["painted_bytes"].as_u64().unwrap_or(0),
            6,
            "all 6 non-zero RGB565 payload bytes sent via GDMA after post-USR DC=HIGH must paint"
        );
        let fb = art
            .bytes
            .as_deref()
            .expect("full inspect returns framebuffer bytes");
        // Verify the first row (x=0..1, y=0) holds the first two RGB565 pixels.
        assert_eq!(&fb[0..4], &[0xF8, 0x00, 0x07, 0xE0]);
        // Verify the second row (x=0..1, y=1, stride 240*2=480 bytes) holds pixels 2..3.
        assert_eq!(&fb[480..484], &[0x00, 0x1F, 0xFF, 0xFF]);

        // Verify CS=HIGH gates subsequent GDMA pixel writes (does not overwrite
        // or paint extra pixels when chip-select is deasserted).
        bus.write_u32(GPIO_OUT_W1TS, 1 << 10).unwrap(); // CS=HIGH (deselected)
        spi2_pio_send(&mut bus, &[0x2C]); // ignored while CS=HIGH
        let extra_pixels: [u8; 8] = [0xAA; 8];
        for (i, &b) in extra_pixels.iter().enumerate() {
            bus.write_u8(buf_addr + i as u64, b).unwrap();
        }
        bus.write_u32(desc_addr, dw0).unwrap();
        bus.write_u32(SPI_CMD, 1u32 << 24).unwrap();
        bus.write_u32(
            GDMA_OUT_LINK_CH0,
            (1u32 << 21) | ((desc_addr as u32) & 0x000F_FFFF),
        )
        .unwrap();
        bus.tick_peripherals();

        let arts_after = bus.display_artifacts_of_format(&["rgb565_be"], &InspectOpts::default());
        assert_eq!(
            arts_after[0].meta["painted_bytes"].as_u64().unwrap_or(0),
            6,
            "CS=HIGH must gate subsequent GDMA pixel writes"
        );
    }

    #[test]
    fn gc9a01_frames_seen_includes_final_frame_without_trailing_noron() {
        let mut bus = make_esp32s3_bus();
        let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
            r#"
schema_version: "1.0"
name: "esp32s3-gc9a01-frames-test"
chip: "esp32s3.yaml"
external_devices:
  - id: "tft"
    type: "gc9a01"
    connection: "spi2_s3"
    config:
      dc_pin: "GPIO13"
      cs_pin: "GPIO10"
"#,
        )
        .expect("parse test manifest");
        let res =
            attach_external_device_universal(&mut bus, &manifest, &manifest.external_devices[0])
                .expect("attach gc9a01");
        assert_eq!(res, UniversalResolution::Attached);

        bus.write_u32(GPIO_ENABLE_W1TS, (1 << 10) | (1 << 13))
            .unwrap();
        // Assert CS=LOW, DC=LOW for init commands.
        bus.write_u32(GPIO_OUT_W1TC, (1 << 10) | (1 << 13)).unwrap();
        spi2_pio_send(&mut bus, &[0x11]); // SLPOUT
        spi2_pio_send(&mut bus, &[0x29]); // DISPON

        // Paint 4 distinct frames using only CASET/RASET/RAMWR + pixel bytes,
        // with NO trailing NORON (0x13) or other command after frame 4.
        let frame_colors: [[u8; 4]; 4] = [
            [0xF8, 0x00, 0xF8, 0x00],
            [0x07, 0xE0, 0x07, 0xE0],
            [0x00, 0x1F, 0x00, 0x1F],
            [0xFF, 0xFF, 0xFF, 0xFF],
        ];
        for pixels in frame_colors {
            bus.write_u32(GPIO_OUT_W1TC, 1 << 13).unwrap(); // DC=LOW
            spi2_pio_send(&mut bus, &[0x2A]); // CASET
            bus.write_u32(GPIO_OUT_W1TS, 1 << 13).unwrap(); // DC=HIGH
            spi2_pio_send(&mut bus, &[0, 0, 0, 1]);
            bus.write_u32(GPIO_OUT_W1TC, 1 << 13).unwrap(); // DC=LOW
            spi2_pio_send(&mut bus, &[0x2B]); // RASET
            bus.write_u32(GPIO_OUT_W1TS, 1 << 13).unwrap(); // DC=HIGH
            spi2_pio_send(&mut bus, &[0, 0, 0, 0]);
            bus.write_u32(GPIO_OUT_W1TC, 1 << 13).unwrap(); // DC=LOW
            spi2_pio_send(&mut bus, &[0x2C]); // RAMWR
            bus.write_u32(GPIO_OUT_W1TS, 1 << 13).unwrap(); // DC=HIGH
            spi2_pio_send(&mut bus, &pixels);
        }

        let arts = bus.display_artifacts_of_format(&["rgb565_be"], &InspectOpts::default());
        assert_eq!(arts.len(), 1);
        let frames_before_noron = arts[0].meta["frames_seen"]
            .as_array()
            .expect("frames_seen array");
        let gens_before_noron = arts[0].meta["generation_history"]
            .as_array()
            .expect("generation_history array");
        assert_eq!(
            frames_before_noron.len(),
            4,
            "all 4 distinct GC9A01 frames must appear in frames_seen without a trailing NORON (0x13)"
        );
        assert_eq!(gens_before_noron.len(), 4);
        let distinct_shas: std::collections::HashSet<&str> = frames_before_noron
            .iter()
            .map(|f| f["sha256"].as_str().expect("sha256"))
            .collect();
        assert_eq!(distinct_shas.len(), 4);

        // Sending a trailing NORON (0x13) afterwards must close cleanly without
        // duplicating frame 4.
        bus.write_u32(GPIO_OUT_W1TC, 1 << 13).unwrap(); // DC=LOW
        spi2_pio_send(&mut bus, &[0x13]); // NORON
        let arts_after_noron =
            bus.display_artifacts_of_format(&["rgb565_be"], &InspectOpts::default());
        let frames_after_noron = arts_after_noron[0].meta["frames_seen"]
            .as_array()
            .expect("frames_seen array");
        assert_eq!(
            frames_after_noron.len(),
            4,
            "trailing NORON (0x13) must not duplicate the already-surfaced 4th frame"
        );
    }
}
