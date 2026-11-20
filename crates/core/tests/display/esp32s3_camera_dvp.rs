// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 camera capture end to end: an `ov3660` part on I2C1, LCD_CAM's
//! CAM slave and a GDMA IN ring, driven with the register sequence
//! `esp32-camera`'s `ll_cam` (esp32s3 target) issues.
//!
//! What it proves is the event contract `cam_hal`'s state machine counts:
//! VSYNC opens a frame, `IN_SUC_EOF` fires once per `CAM_REC_DATA_BYTELEN + 1`
//! bytes, the next VSYNC closes it — and that the descriptor ring then holds
//! exactly the bytes the sensor put on D[7:0] for the window and format the
//! firmware programmed over SCCB.
//!
//! The whole ring is DMA-owned once and `IN_CHECK_OWNER` is clear, as in
//! `ll_cam_dma_init`; the second half-buffer lands in descriptor 1 and the next
//! frame wraps back to descriptor 0 even though the first lap returned its
//! owner bit to the CPU.
//!
//! The register-level tests attach the sensor themselves (SCCB on I2C1, DVP
//! through `SystemBus::attach_dvp_sensor`) and keep a second SCCB handle on the
//! same sensor to play the part of `esp32-camera`'s SCCB master. The kit test
//! attaches it from a manifest and reaches it only through the generic
//! stimulus path, which is how the `ov3660` kit is wired in a real lab.

use std::sync::Arc;

use labwired_config::SystemManifest;
use labwired_core::bus::SystemBus;
use labwired_core::cpu::XtensaLx7;
use labwired_core::peripherals::components::{Ov3660, Ov3660Sccb, OV3660_SCCB_ADDR};
use labwired_core::peripherals::esp32s3::lcd_cam::LCD_CAM_BASE;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::sim_input::SimInput;
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3Opts,
};
use labwired_core::{Bus, Machine};

// ── LCD_CAM ──────────────────────────────────────────────────────────────
const CAM_CTRL1: u64 = LCD_CAM_BASE as u64 + 0x08;
const LC_DMA_INT_ENA: u64 = LCD_CAM_BASE as u64 + 0x64;
const LC_DMA_INT_RAW: u64 = LCD_CAM_BASE as u64 + 0x68;
const LC_DMA_INT_CLR: u64 = LCD_CAM_BASE as u64 + 0x70;
const CAM_START: u32 = 1 << 29;
const CAM_RESET: u32 = 1 << 30;
const INT_CAM_VSYNC: u32 = 1 << 2;

// ── GDMA channel 0, IN direction ─────────────────────────────────────────
const GDMA_BASE: u64 = 0x6003_F000;
const IN_CONF0: u64 = GDMA_BASE;
const IN_CONF1: u64 = GDMA_BASE + 0x04;
const IN_INT_RAW: u64 = GDMA_BASE + 0x08;
const IN_INT_ENA: u64 = GDMA_BASE + 0x10;
const IN_INT_CLR: u64 = GDMA_BASE + 0x14;
const IN_LINK: u64 = GDMA_BASE + 0x20;
const IN_PERI_SEL: u64 = GDMA_BASE + 0x48;
const IN_RST: u32 = 1 << 0;
const IN_LINK_STOP: u32 = 1 << 21;
const IN_LINK_START: u32 = 1 << 22;
const IN_SUC_EOF: u32 = 1 << 1;
const PERI_SEL_LCD_CAM: u32 = 5;

// Ring: two 128-byte nodes over one 256-byte buffer (a "half buffer" each).
const DESC0: u64 = 0x3FC9_0000;
const DESC1: u64 = 0x3FC9_0010;
const BUF: u64 = 0x3FC9_1000;
const NODE: u32 = 128;
const OWNER_DMA: u32 = 1 << 31;

// The window the "firmware" programs: 16 × 8 RGB565 = 256 bytes = 2 nodes.
const W: u8 = 16;
const H: u8 = 8;
const FRAME_BYTES: usize = W as usize * H as usize * 2;

struct Rig {
    machine: Machine<XtensaLx7>,
    /// The test's SCCB master: a second handle on the sensor the I2C1
    /// controller also holds (`None` for a manifest-attached sensor).
    sccb: Option<Ov3660Sccb>,
}

impl Rig {
    fn bus(&mut self) -> &mut SystemBus {
        &mut self.machine.bus
    }

    fn step(&mut self) {
        self.machine
            .step()
            .expect("parked machine advances cleanly");
    }

    fn rd(&mut self, addr: u64) -> u32 {
        self.bus().read_u32(addr).unwrap()
    }

    fn wr(&mut self, addr: u64, v: u32) {
        self.bus().write_u32(addr, v).unwrap();
    }

    /// One SCCB transaction — the `start`/`write`/`read`/`stop` calls the I2C
    /// controller makes for `SCCB_Write16` / `SCCB_Read16`.
    fn with_sccb<R>(&mut self, f: impl FnOnce(&mut Ov3660Sccb) -> R) -> R {
        f(self
            .sccb
            .as_mut()
            .expect("this rig attached the sensor itself"))
    }

    fn sccb_write(&mut self, reg: u16, v: u8) {
        self.with_sccb(|d| {
            d.start();
            d.write((reg >> 8) as u8);
            d.write(reg as u8);
            d.write(v);
            d.stop();
        });
    }

    fn sccb_read(&mut self, reg: u16) -> u8 {
        self.with_sccb(|d| {
            d.start();
            d.write((reg >> 8) as u8);
            d.write(reg as u8);
            d.stop();
            d.start();
            let v = d.read();
            d.stop();
            v
        })
    }

    /// Step until `addr & mask` is set, or panic after `limit` steps.
    fn wait_for(&mut self, addr: u64, mask: u32, limit: usize, what: &str) -> usize {
        for n in 0..limit {
            if self.rd(addr) & mask != 0 {
                return n;
            }
            self.step();
        }
        panic!("{what}: not seen within {limit} steps");
    }
}

fn machine(bus: SystemBus, cpu: XtensaLx7, sccb: Option<Ov3660Sccb>) -> Rig {
    let mut cpu = cpu;
    cpu.halted = true;
    Rig {
        machine: Machine::new(cpu, bus),
        sccb,
    }
}

/// An S3 with an OV3660 attached by hand: SCCB on I2C1, DVP on LCD_CAM.
fn build() -> Rig {
    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let sensor = Arc::new(Ov3660::new());
    bus.attach_i2c_slave(
        "i2c1",
        Box::new(Ov3660Sccb::new(OV3660_SCCB_ADDR, sensor.clone())),
    )
    .expect("attach SCCB to i2c1");
    assert!(
        bus.attach_dvp_sensor(sensor.clone()),
        "the S3 has a camera interface to take the sensor"
    );
    machine(
        bus,
        wiring.cpu,
        Some(Ov3660Sccb::new(OV3660_SCCB_ADDR, sensor)),
    )
}

/// An S3 with an OV3660 attached the way a lab attaches it: from a manifest,
/// through the `ov3660` kit.
fn build_from_manifest() -> Rig {
    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "s3-ov3660"
chip: "../chips/esp32s3.yaml"
external_devices:
  - id: "cam"
    type: "ov3660"
    connection: "i2c1"
board_io: []
"#,
    )
    .expect("parse manifest");
    attach_esp32_external_devices(&mut bus, &manifest).expect("attach ov3660");
    machine(bus, wiring.cpu, None)
}

/// `ll_cam_dma_init` + the ring `allocate_dma_descriptors` builds.
fn arm_ring(rig: &mut Rig) {
    for (desc, next, off) in [(DESC0, DESC1, 0u64), (DESC1, DESC0, u64::from(NODE))] {
        rig.wr(desc, OWNER_DMA | NODE);
        rig.wr(desc + 4, (BUF + off) as u32);
        rig.wr(desc + 8, next as u32);
    }
    rig.wr(IN_INT_CLR, !0);
    rig.wr(IN_INT_ENA, 0);
    rig.wr(IN_CONF0, IN_RST);
    rig.wr(IN_CONF0, 0);
    rig.wr(IN_CONF1, 0); // IN_CHECK_OWNER clear
    rig.wr(IN_PERI_SEL, PERI_SEL_LCD_CAM);
}

/// `ll_cam_start`.
fn ll_cam_start(rig: &mut Rig) {
    let ctrl1 = rig.rd(CAM_CTRL1) & !CAM_START;
    rig.wr(CAM_CTRL1, ctrl1);
    rig.wr(IN_INT_CLR, !0);
    rig.wr(IN_INT_ENA, IN_SUC_EOF);
    rig.wr(IN_CONF0, IN_RST);
    rig.wr(IN_CONF0, 0);
    rig.wr(CAM_CTRL1, (NODE - 1) | CAM_RESET);
    rig.wr(IN_LINK, (DESC0 as u32 & 0xF_FFFF) | IN_LINK_START);
    rig.wr(CAM_CTRL1, (NODE - 1) | CAM_START);
}

/// `ll_cam_stop`.
fn ll_cam_stop(rig: &mut Rig) {
    rig.wr(IN_INT_ENA, 0);
    rig.wr(IN_INT_CLR, IN_SUC_EOF);
    let ctrl1 = rig.rd(CAM_CTRL1) & !CAM_START;
    rig.wr(CAM_CTRL1, ctrl1);
    rig.wr(IN_LINK, IN_LINK_STOP);
}

fn ring_bytes(rig: &mut Rig) -> Vec<u8> {
    (0..FRAME_BYTES as u64)
        .map(|i| rig.bus().read_u8(BUF + i).unwrap())
        .collect()
}

/// What the sensor puts on the pins for a 16-wide colour-bar frame: two
/// pixels per bar, RGB565 big-endian, the same eight bars on every line.
fn expected_bars_frame() -> Vec<u8> {
    const BARS: [u16; 8] = [
        0xFFFF, 0xFFE0, 0x07FF, 0x07E0, 0xF81F, 0xF800, 0x001F, 0x0000,
    ];
    let mut out = Vec::with_capacity(FRAME_BYTES);
    for _ in 0..H {
        for x in 0..W as usize {
            out.extend_from_slice(&BARS[x * 8 / W as usize].to_be_bytes());
        }
    }
    out
}

/// Software reset, then the window and format `set_framesize` /
/// `set_pixformat(RGB565)` program.
fn program_window(rig: &mut Rig) {
    rig.sccb_write(0x3008, 0x82);
    for (reg, v) in [
        (0x3808, 0),
        (0x3809, W),
        (0x380A, 0),
        (0x380B, H),
        (0x4300, 0x61),
    ] {
        rig.sccb_write(reg, v);
    }
}

/// `ll_cam_config` + `cam_start()`: REC_DATA_BYTELEN and CAM_START, the DMA
/// ring, and the VSYNC interrupt the task waits on.
fn configure_capture(rig: &mut Rig) {
    rig.wr(CAM_CTRL1, (4096 - 1) | CAM_START);
    arm_ring(rig);
    rig.wr(LC_DMA_INT_ENA, INT_CAM_VSYNC);
}

/// One pass of `cam_task`: in CAM_STATE_IDLE wait for VSYNC and
/// `ll_cam_start`; in CAM_STATE_READ_BUF acknowledge each `IN_SUC_EOF` (the
/// DMA ISR's `int_clr`) until the closing VSYNC, then `ll_cam_stop`. Returns
/// the number of EOF events seen. The closing VSYNC is left raw: it is the
/// next frame's opening one.
fn capture_frame(rig: &mut Rig) -> usize {
    rig.wait_for(
        LC_DMA_INT_RAW,
        INT_CAM_VSYNC,
        2_000_000,
        "frame-start VSYNC",
    );
    rig.wr(LC_DMA_INT_CLR, INT_CAM_VSYNC);
    ll_cam_start(rig);

    let mut eofs = 0;
    for _ in 0..2_000_000 {
        if rig.rd(IN_INT_RAW) & IN_SUC_EOF != 0 {
            rig.wr(IN_INT_CLR, IN_SUC_EOF);
            eofs += 1;
        }
        if rig.rd(LC_DMA_INT_RAW) & INT_CAM_VSYNC != 0 {
            ll_cam_stop(rig);
            return eofs;
        }
        assert!(
            eofs <= FRAME_BYTES / NODE as usize,
            "more EOFs than half buffers"
        );
        rig.step();
    }
    panic!("closing VSYNC not seen ({eofs} EOFs so far)");
}

#[test]
fn ov3660_frames_reach_memory_in_the_order_cam_hal_counts() {
    let mut rig = build();

    // camera_probe → ov3660_detect.
    assert_eq!(rig.sccb_read(0x300A), 0x36);
    assert_eq!(rig.sccb_read(0x300B), 0x60);

    program_window(&mut rig);
    configure_capture(&mut rig);

    for frame in 0..2 {
        assert_eq!(
            capture_frame(&mut rig),
            FRAME_BYTES / NODE as usize,
            "frame {frame}: one IN_SUC_EOF per REC_DATA_BYTELEN + 1 bytes before the closing VSYNC"
        );
        assert_eq!(
            ring_bytes(&mut rig),
            expected_bars_frame(),
            "frame {frame} contents"
        );
    }
}

#[test]
fn stimulus_changes_what_is_captured() {
    let mut rig = build();
    program_window(&mut rig);
    // Stimulus: flat, 0 % brightness → an all-black frame.
    rig.with_sccb(|d| {
        d.set_input("pattern", 3.0).unwrap();
        d.set_input("brightness", 0.0).unwrap();
    });
    configure_capture(&mut rig);
    for i in 0..FRAME_BYTES as u64 {
        rig.bus().write_u8(BUF + i, 0xAA).unwrap();
    }
    capture_frame(&mut rig);
    assert!(
        ring_bytes(&mut rig).iter().all(|&b| b == 0),
        "stimulus reached the pixels"
    );
}

/// The guard that keeps `cam_hal`'s count intact: a second EOF block is not
/// written over an `IN_SUC_EOF` the CPU has not acknowledged, and the frame
/// does not close (no VSYNC) while bytes are still owed to the ring.
#[test]
fn an_unacknowledged_eof_holds_the_stream_instead_of_merging_two() {
    let mut rig = build();
    program_window(&mut rig);
    configure_capture(&mut rig);
    rig.wait_for(LC_DMA_INT_RAW, INT_CAM_VSYNC, 2_000_000, "VSYNC");
    rig.wr(LC_DMA_INT_CLR, INT_CAM_VSYNC);
    ll_cam_start(&mut rig);
    rig.wait_for(IN_INT_RAW, IN_SUC_EOF, 2_000_000, "first EOF");
    for _ in 0..200_000 {
        rig.step();
    }
    assert_eq!(
        rig.rd(LC_DMA_INT_RAW) & INT_CAM_VSYNC,
        0,
        "no VSYNC while half the frame is still owed"
    );
    assert_eq!(
        rig.bus().read_u8(BUF + u64::from(NODE)).unwrap(),
        0,
        "second half-buffer untouched"
    );
    // The ISR acknowledges; the rest of the frame then arrives.
    rig.wr(IN_INT_CLR, IN_SUC_EOF);
    rig.wait_for(IN_INT_RAW, IN_SUC_EOF, 2_000_000, "second EOF");
    rig.wr(IN_INT_CLR, IN_SUC_EOF);
    rig.wait_for(LC_DMA_INT_RAW, INT_CAM_VSYNC, 2_000_000, "closing VSYNC");
}

/// The kit wires both halves: the manifest id reaches the sensor through the
/// generic stimulus path, and its frames reach the ring through LCD_CAM. No
/// SCCB here, so this is the sensor's out-of-reset output: YUV422 YUYV.
#[test]
fn the_ov3660_kit_binds_sccb_stimulus_and_dvp_from_a_manifest() {
    let mut rig = build_from_manifest();
    rig.bus()
        .set_input(Some("cam"), "brightness", 0.0)
        .expect("stimulus resolves the manifest id");
    configure_capture(&mut rig);
    rig.wait_for(LC_DMA_INT_RAW, INT_CAM_VSYNC, 2_000_000, "VSYNC");
    rig.wr(LC_DMA_INT_CLR, INT_CAM_VSYNC);
    ll_cam_start(&mut rig);
    rig.wait_for(IN_INT_RAW, IN_SUC_EOF, 2_000_000, "first EOF");
    let first_block: Vec<u8> = (0..u64::from(NODE))
        .map(|i| rig.bus().read_u8(BUF + i).unwrap())
        .collect();
    // Black in YUYV: Y = 0, U = V = 128.
    assert_eq!(first_block, [0u8, 128, 0, 128].repeat(NODE as usize / 4));
}

// ── I²C over the GPIO matrix ─────────────────────────────────────────────
const I2C1_BASE: u64 = 0x6002_7000;
const I2C_CTR: u64 = I2C1_BASE + 0x04;
const I2C_SLAVE_ADDR: u64 = I2C1_BASE + 0x10;
const I2C_DATA: u64 = I2C1_BASE + 0x1C;
const I2C_INT_RAW: u64 = I2C1_BASE + 0x20;
const I2C_INT_CLR: u64 = I2C1_BASE + 0x24;
const I2C_CMD0: u64 = I2C1_BASE + 0x58;
const I2C_TRANS_START: u32 = 1 << 5;
const I2C_INT_NACK: u32 = 1 << 10;
const I2C_INT_TRANS_COMPLETE: u32 = 1 << 7;
const GPIO_FUNC_OUT_SEL: u64 = 0x6000_4554;
const I2CEXT1_SCL_OUT: u32 = 91;
const I2CEXT1_SDA_OUT: u32 = 92;

/// `SCCB_Read16(0x3C, reg)` as `esp32-camera` issues it on I2C1: a write of
/// the register pointer, a repeated start, a one-byte read. Returns the byte
/// and whether any address frame went unacknowledged.
fn i2c1_read16(rig: &mut Rig, reg: u16) -> (u8, bool) {
    const RSTART: u32 = 6 << 11;
    const WRITE: u32 = 1 << 11;
    const READ: u32 = 3 << 11;
    const STOP: u32 = 2 << 11;
    const ACK_CHECK: u32 = 1 << 8;
    let addr = u32::from(OV3660_SCCB_ADDR) << 1;
    rig.wr(I2C_INT_CLR, u32::MAX);
    rig.wr(I2C_SLAVE_ADDR, 0);
    for b in [addr, u32::from(reg >> 8), u32::from(reg & 0xFF), addr | 1] {
        rig.wr(I2C_DATA, b);
    }
    for (i, cmd) in [
        RSTART,
        WRITE | ACK_CHECK | 3,
        RSTART,
        WRITE | ACK_CHECK | 1,
        READ | 1,
        STOP,
    ]
    .into_iter()
    .enumerate()
    {
        rig.wr(I2C_CMD0 + 4 * i as u64, cmd);
    }
    let ctr = rig.rd(I2C_CTR);
    rig.wr(I2C_CTR, ctr | I2C_TRANS_START);
    rig.wait_for(I2C_INT_RAW, I2C_INT_TRANS_COMPLETE, 1_000_000, "I2C1 STOP");
    let nacked = rig.rd(I2C_INT_RAW) & I2C_INT_NACK != 0;
    (rig.rd(I2C_DATA) as u8, nacked)
}

/// The canvas names the controller its pin table offers (`i2c0`) and the
/// pads the camera is soldered to; `esp32-camera` opens I2C1 and routes it to
/// those pads. The camera answers I2C1 exactly while the matrix sends I2C1's
/// SCL/SDA to its pads — as on silicon — and not before.
#[test]
fn the_ov3660_answers_whichever_i2c_the_matrix_routes_to_its_pads() {
    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "s3-ov3660-i2c0"
chip: "../chips/esp32s3.yaml"
external_devices:
  - id: "cam"
    type: "ov3660"
    connection: "i2c0"
    route:
      sda: "GPIO4"
      scl: "GPIO5"
board_io: []
"#,
    )
    .expect("parse manifest");
    attach_esp32_external_devices(&mut bus, &manifest).expect("attach ov3660");
    let mut rig = machine(bus, wiring.cpu, None);

    let (_, nacked) = i2c1_read16(&mut rig, 0x300A);
    assert!(nacked, "I2C1 not routed to the camera's pads: no ACK");

    rig.wr(GPIO_FUNC_OUT_SEL + 4 * 5, I2CEXT1_SCL_OUT);
    rig.wr(GPIO_FUNC_OUT_SEL + 4 * 4, I2CEXT1_SDA_OUT);
    let (pid_h, nacked) = i2c1_read16(&mut rig, 0x300A);
    assert!(!nacked, "I2C1 routed to GPIO5/GPIO4: the camera answers");
    let (pid_l, _) = i2c1_read16(&mut rig, 0x300B);
    assert_eq!(u16::from_be_bytes([pid_h, pid_l]), 0x3660);
}

// ── JPEG through LCD_CAM and GDMA ────────────────────────────────────────
// `ll_cam_dma_sizes` in JPEG mode: 1 KiB half buffers (REC_DATA_BYTELEN 1023).
const JPEG_BLOCK: u32 = 1024;
const JPEG_DESC: u64 = 0x3FC9_3000;
const JPEG_BUF: u64 = 0x3FC9_4000;
const JPEG_NODES: u64 = 4;

/// A 64×48 window in JPEG mode reaches memory as a whole JPEG: SOI at byte 0
/// of the first block, one `IN_SUC_EOF` per full 1 KiB block, EOI inside the
/// last block the driver counts, and the bytes in RAM decode to the window.
#[test]
fn ov3660_jpeg_frames_reach_memory_as_a_decodable_jpeg() {
    let mut rig = build();
    rig.sccb_write(0x3008, 0x82);
    for (reg, v) in [(0x3808, 0), (0x3809, 64), (0x380A, 0), (0x380B, 48)] {
        rig.sccb_write(reg, v);
    }
    // set_pixformat(JPEG): YUV422 core format + the compression enable;
    // set_quality(12).
    rig.sccb_write(0x4300, 0x30);
    rig.sccb_write(0x3821, 0x20);
    rig.sccb_write(0x4407, 12);

    for n in 0..JPEG_NODES {
        let desc = JPEG_DESC + n * 16;
        let next = JPEG_DESC + ((n + 1) % JPEG_NODES) * 16;
        rig.wr(desc, OWNER_DMA | JPEG_BLOCK);
        rig.wr(desc + 4, (JPEG_BUF + n * u64::from(JPEG_BLOCK)) as u32);
        rig.wr(desc + 8, next as u32);
    }
    rig.wr(IN_INT_CLR, !0);
    rig.wr(IN_CONF1, 0);
    rig.wr(IN_PERI_SEL, PERI_SEL_LCD_CAM);
    rig.wr(CAM_CTRL1, (JPEG_BLOCK - 1) | CAM_START);
    rig.wr(LC_DMA_INT_ENA, INT_CAM_VSYNC);

    rig.wait_for(
        LC_DMA_INT_RAW,
        INT_CAM_VSYNC,
        2_000_000,
        "frame-start VSYNC",
    );
    rig.wr(LC_DMA_INT_CLR, INT_CAM_VSYNC);
    // ll_cam_start, JPEG half-buffer size.
    rig.wr(IN_INT_CLR, !0);
    rig.wr(IN_INT_ENA, IN_SUC_EOF);
    rig.wr(IN_CONF0, IN_RST);
    rig.wr(IN_CONF0, 0);
    rig.wr(CAM_CTRL1, (JPEG_BLOCK - 1) | CAM_RESET);
    rig.wr(IN_LINK, (JPEG_DESC as u32 & 0xF_FFFF) | IN_LINK_START);
    rig.wr(CAM_CTRL1, (JPEG_BLOCK - 1) | CAM_START);

    let mut eofs = 0u64;
    let mut closed = false;
    for _ in 0..4_000_000 {
        if rig.rd(IN_INT_RAW) & IN_SUC_EOF != 0 {
            rig.wr(IN_INT_CLR, IN_SUC_EOF);
            eofs += 1;
        }
        if rig.rd(LC_DMA_INT_RAW) & INT_CAM_VSYNC != 0 {
            ll_cam_stop(&mut rig);
            closed = true;
            break;
        }
        rig.step();
    }
    assert!(closed, "closing VSYNC not seen ({eofs} EOFs)");
    assert!(eofs < JPEG_NODES, "the frame fits the ring ({eofs} EOFs)");

    // What cam_hal does with it in PSRAM mode: (EOFs + 1) blocks, SOI at the
    // start, EOI searched forward in the last block plus one byte.
    let reported = (eofs + 1) * u64::from(JPEG_BLOCK);
    let ram: Vec<u8> = (0..reported)
        .map(|i| rig.bus().read_u8(JPEG_BUF + i).unwrap())
        .collect();
    assert_eq!(
        &ram[..3],
        &[0xFF, 0xD8, 0xFF],
        "SOI at byte 0 of the first block"
    );
    let window = reported as usize - (JPEG_BLOCK as usize + 1);
    let eoi = ram[window..]
        .windows(2)
        .position(|w| w == [0xFF, 0xD9])
        .map(|p| p + window)
        .expect("EOI where esp32-camera looks for it");
    let mut dec = jpeg_decoder::Decoder::new(&ram[..eoi + 2]);
    let px = dec.decode().expect("the bytes in RAM are a valid JPEG");
    let info = dec.info().unwrap();
    assert_eq!((info.width, info.height), (64, 48));
    // Out-of-reset scene: colour bars; the first column is the white bar.
    assert!(
        px[..3].iter().all(|&c| c > 220),
        "white bar first: {:?}",
        &px[..3]
    );
}
