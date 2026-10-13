//! Host protocol tests for functional RK3x v1; not evidence of Linux execution or wire timing.
use labwired_core::{
    bus::SystemBus,
    peripherals::{
        armv7::rk3x_i2c::Rk3xI2c,
        components::{declarative_i2c::GenericI2cDevice, supply::UnpoweredI2cDevice},
        device::I2cDevice,
    },
    Bus, Machine, Peripheral,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
const BASE: u64 = 0xff460000;
const EN: u32 = 1;
const START: u32 = 8;
const STOP: u32 = 16;
const ACTACK: u32 = 64;
const REG_TX: u32 = 2;
const RX: u32 = 4;
const LASTACK: u32 = 32;
const VALID: u32 = 1 << 24;
const D: u64 = 0xff1f1000;
const C: u64 = 0xff1f2000;
#[derive(Debug, Clone, PartialEq)]
enum Call {
    Select(u8),
    Start,
    Write(u8),
    Read(u8),
    Stop,
    Time(u64),
}
#[derive(Default)]
struct Probe {
    calls: Vec<Call>,
    next: u8,
    pins_drained: bool,
    cut_after: Option<Call>,
}
struct Counted {
    probe: Arc<Mutex<Probe>>,
    powered: Arc<AtomicBool>,
}
impl Counted {
    fn record(&self, c: Call) {
        let mut p = self.probe.lock().unwrap();
        if p.cut_after.as_ref() == Some(&c) {
            self.powered.store(false, Ordering::Relaxed);
        }
        p.calls.push(c);
    }
}
impl labwired_core::sim_input::SimInput for Counted {
    fn input_channels(&self) -> &[labwired_core::sim_input::InputChannel] {
        &[]
    }
    fn set_input(
        &mut self,
        key: &str,
        _: f64,
    ) -> Result<(), labwired_core::sim_input::SimInputError> {
        Err(labwired_core::sim_input::SimInputError::UnknownChannel(
            key.into(),
        ))
    }
    fn component_id(&self) -> Option<&str> {
        Some("probe")
    }
}
impl I2cDevice for Counted {
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn labwired_core::sim_input::SimInput> {
        Some(self)
    }
    fn take_pin_drives(&mut self) -> Vec<(String, bool)> {
        let mut p = self.probe.lock().unwrap();
        if p.pins_drained {
            vec![]
        } else {
            p.pins_drained = true;
            vec![("irq".into(), true)]
        }
    }
    fn address(&self) -> u8 {
        0x53
    }
    fn claims_address(&self, a: u8) -> bool {
        a == 0x53 && self.powered.load(Ordering::Relaxed)
    }
    fn select_address(&mut self, a: u8) {
        self.record(Call::Select(a));
    }
    fn start(&mut self) {
        self.record(Call::Start);
    }
    fn write(&mut self, b: u8) {
        self.record(Call::Write(b));
    }
    fn read(&mut self) -> u8 {
        let mut p = self.probe.lock().unwrap();
        let b = p.next;
        p.next = p.next.wrapping_add(1);
        drop(p);
        self.record(Call::Read(b));
        b
    }
    fn stop(&mut self) {
        self.record(Call::Stop);
    }
    fn advance_time_us(&mut self, us: u64) {
        self.probe.lock().unwrap().calls.push(Call::Time(us));
    }
    fn artifacts(
        &self,
        id: &str,
        opts: &labwired_core::inspect::InspectOpts,
    ) -> Vec<labwired_core::inspect::Artifact> {
        vec![labwired_core::inspect::Artifact {
            kind: "probe_calls".into(),
            id: id.into(),
            meta: serde_json::json!({"calls": self.probe.lock().unwrap().calls.len()}),
            bytes: opts.include_bytes.then(|| vec![0x53]),
        }]
    }
    fn logs(&self) -> Vec<labwired_core::peripheral_log::PeripheralLog> {
        vec![labwired_core::peripheral_log::PeripheralLog::new(
            "probe",
            vec!["genuine inner log".into()],
        )]
    }
}
fn counted() -> (Box<dyn I2cDevice>, Arc<Mutex<Probe>>, Arc<AtomicBool>) {
    let p = Arc::new(Mutex::new(Probe::default()));
    let on = Arc::new(AtomicBool::new(true));
    (
        Box::new(Counted {
            probe: p.clone(),
            powered: on.clone(),
        }),
        p,
        on,
    )
}
fn bus() -> SystemBus {
    let mut b = SystemBus::new();
    b.cpu_hz = 24_000_000;
    b.configure_armv7_rv1103().unwrap();
    b.configure_armv7_rv1103_i2c3().unwrap();
    b
}
fn put(b: &mut SystemBus, o: u64, v: u32) {
    b.write_u32(BASE + o, v).unwrap();
}
fn get(b: &mut SystemBus, o: u64) -> u32 {
    b.read_u32(BASE + o).unwrap()
}
fn begin(b: &mut SystemBus, mode: u32) {
    put(b, 0x1c, 0x7f);
    put(b, 0x18, 0x10);
    put(b, 0, EN | mode | START | ACTACK);
    assert_eq!(get(b, 0x1c), 0x10);
    put(b, 0x1c, 0x10);
    put(b, 0, EN | mode | ACTACK);
    put(b, 0x18, if mode == 0 { 0x44 } else { 0x48 });
}
fn stop(b: &mut SystemBus) {
    put(b, 0x18, 0x20);
    let c = get(b, 0);
    put(b, 0, c | STOP);
    assert_eq!(get(b, 0x1c), 0x20);
    put(b, 0x1c, 0x20);
    put(b, 0, c & !STOP);
}
fn fifo(b: &mut SystemBus, bytes: &[u8]) {
    for (i, chunk) in bytes.chunks(4).enumerate() {
        let mut a = [0; 4];
        a[..chunk.len()].copy_from_slice(chunk);
        put(b, 0x100 + i as u64 * 4, u32::from_le_bytes(a));
    }
}
fn tx(b: &mut SystemBus, data: &[u8]) {
    begin(b, 0);
    let mut v = vec![0xa6];
    v.extend_from_slice(data);
    fifo(b, &v);
    put(b, 0x10, v.len() as u32);
    assert_eq!(get(b, 0x1c), 4);
    put(b, 0x1c, 4);
    stop(b);
}
fn read_register(b: &mut SystemBus, prefix: &[u8], count: usize) -> Vec<u8> {
    put(b, 8, 0xa6 | VALID);
    let mut a = [0; 4];
    a[..prefix.len()].copy_from_slice(prefix);
    let valid = prefix
        .iter()
        .enumerate()
        .fold(0, |v, (i, _)| v | (1 << (24 + i)));
    put(b, 12, u32::from_le_bytes(a) | valid);
    begin(b, REG_TX);
    put(b, 0, EN | REG_TX | ACTACK | LASTACK);
    put(b, 0x14, count as u32);
    assert_eq!(get(b, 0x1c), 8);
    let out = (0..count)
        .map(|i| ((get(b, 0x200 + (i / 4) as u64 * 4) >> ((i % 4) * 8)) & 255) as u8)
        .collect();
    put(b, 0x1c, 8);
    stop(b);
    out
}
fn controller(b: &mut SystemBus) -> &mut Rk3xI2c {
    let i = b.find_peripheral_index_by_name("i2c3").unwrap();
    b.peripherals[i]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut()
        .unwrap()
}
fn canonical_manifest(powered: bool) -> labwired_config::SystemManifest {
    serde_yaml::from_str(&format!("name: basket\nchip: rv1103\nexternal_devices:\n  - id: basket:adxl\n    type: adxl345\n    connection: i2c3\n    route: {{sda: GPIO1_C2, scl: GPIO1_C3}}\n    config: {{i2c_address: 0x53, powered: {powered}}}\n")).unwrap()
}
fn attach_canonical(b: &mut SystemBus, powered: bool) {
    let m = canonical_manifest(powered);
    b.attach_manifest_external_devices(&m).unwrap();
}
#[test]
fn normal_i2c_attach_accepts_rv1103_i2c3() {
    let mut b = bus();
    let (s, _, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    assert!(b.attach_i2c_slave("missing", counted().0).is_err());
    assert!(b.attach_i2c_slave("uart2", counted().0).is_err());
}
#[test]
fn opt_in_install_requires_fabric_and_rejects_duplicates_and_overlaps_atomically() {
    let mut b = SystemBus::new();
    assert!(b.configure_armv7_rv1103_i2c3().is_err());
    let mut b = bus();
    let n = b.peripherals.len();
    assert!(b.configure_armv7_rv1103_i2c3().is_err());
    assert_eq!(b.peripherals.len(), n);
    let mut b = SystemBus::new();
    b.cpu_hz = 24_000_000;
    b.configure_armv7_rv1103().unwrap();
    let g = b.irq_fabric.armv7.as_ref().unwrap().gic.clone();
    b.add_peripheral(
        "conflict",
        BASE + 0x800,
        0x100,
        None,
        Box::new(Rk3xI2c::new(g, 53)),
    );
    let n = b.peripherals.len();
    assert!(b.configure_armv7_rv1103_i2c3().is_err());
    assert_eq!(b.peripherals.len(), n);
}
#[test]
fn start_irq53_is_level_masked_and_w1c_and_waits_for_count() {
    let mut b = bus();
    let (s, p, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    b.write_u32(D + 0x104, 1 << 21).unwrap();
    b.write_u32(D, 1).unwrap();
    b.write_u32(C + 4, 255).unwrap();
    b.write_u32(C, 1).unwrap();
    put(&mut b, 0x18, 0x10);
    put(&mut b, 0, EN | START | ACTACK | 0x5500);
    assert!(p.lock().unwrap().calls.is_empty());
    assert_eq!(b.arm_irq_fiq_levels(), Some((true, false)));
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 53);
    put(&mut b, 0x18, 0);
    b.write_u32(C + 0x10, 53).unwrap();
    assert_eq!(b.arm_irq_fiq_levels(), Some((false, false)));
    assert_eq!(get(&mut b, 0x1c), 0x10);
    put(&mut b, 0x18, 0x10);
    assert_eq!(b.arm_irq_fiq_levels(), Some((true, false)));
    put(&mut b, 0x1c, 0x40);
    assert_eq!(get(&mut b, 0x1c), 0x10);
    put(&mut b, 0x1c, 0x10);
    assert_eq!(b.arm_irq_fiq_levels(), Some((false, false)));
    put(&mut b, 0, EN | ACTACK | 0x5500);
    put(&mut b, 0x18, 4);
    fifo(&mut b, &[0xa6, 0x2d, 8]);
    put(&mut b, 0x10, 3);
    assert_eq!(get(&mut b, 0x1c), 4);
    assert_eq!(get(&mut b, 0) & 0xff00, 0x5500);
    assert_eq!(b.arm_irq_fiq_levels(), Some((true, false)));
    put(&mut b, 0x1c, 4);
    assert_eq!(b.arm_irq_fiq_levels(), Some((false, false)));
}
#[test]
fn tx_first_chunk_strips_address_and_31_plus_32_continues_without_restart() {
    let mut b = bus();
    let (s, p, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    begin(&mut b, 0);
    let mut first = vec![0xa6];
    first.extend(0..31);
    fifo(&mut b, &first);
    put(&mut b, 0x10, 32);
    put(&mut b, 0x1c, 4);
    fifo(&mut b, &(31..63).collect::<Vec<_>>());
    put(&mut b, 0x10, 32);
    assert_eq!(get(&mut b, 0x20), 32);
    put(&mut b, 0x1c, 4);
    stop(&mut b);
    let mut expected = vec![Call::Select(0x53), Call::Start];
    expected.extend((0..63).map(Call::Write));
    expected.push(Call::Stop);
    assert_eq!(p.lock().unwrap().calls, expected);
}
#[test]
fn combined_read_sends_up_to_three_prefix_bytes_once_then_restarts_same_target() {
    for prefix in [&[0x32][..], &[1, 2][..], &[1, 2, 3][..]] {
        let mut b = bus();
        let (s, p, _) = counted();
        b.attach_i2c_slave("i2c3", s).unwrap();
        assert_eq!(read_register(&mut b, prefix, 2), [0, 1]);
        let mut e = vec![Call::Select(0x53), Call::Start];
        e.extend(prefix.iter().copied().map(Call::Write));
        e.push(Call::Start);
        e.extend([Call::Read(0), Call::Read(1), Call::Stop]);
        assert_eq!(p.lock().unwrap().calls, e);
    }
}
#[test]
fn plain_raw_read_and_32_plus_one_rx_continue_the_same_cursor_and_lastack_is_not_nak() {
    let mut b = bus();
    let (s, p, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    put(&mut b, 8, 0xa7 | VALID);
    put(&mut b, 12, 0);
    begin(&mut b, REG_TX);
    put(&mut b, 0x14, 32);
    assert_eq!(get(&mut b, 0x1c), 8);
    assert_eq!(get(&mut b, 0x200), 0x03020100);
    put(&mut b, 0x1c, 8);
    put(&mut b, 0, EN | RX | ACTACK | LASTACK);
    put(&mut b, 0x14, 1);
    assert_eq!(get(&mut b, 0x1c), 8);
    assert_eq!(get(&mut b, 0x200), 32);
    put(&mut b, 0x1c, 8);
    stop(&mut b);
    let mut e = vec![Call::Select(0x53), Call::Start];
    e.extend((0..33).map(Call::Read));
    e.push(Call::Stop);
    assert_eq!(p.lock().unwrap().calls, e);
}
#[test]
fn canonical_kit_adxl345_devid_axes_two_machine_stimuli_and_genuine_bus_trace() {
    let mut b = bus();
    attach_canonical(&mut b, true);
    let mut m = Machine::new(labwired_core::cpu::cortex_m::CortexM::new(), b);
    assert_eq!(
        m.list_inputs()
            .iter()
            .map(|(id, ch)| (id.as_str(), ch.key.as_ref()))
            .collect::<Vec<_>>(),
        [
            ("basket:adxl", "x"),
            ("basket:adxl", "y"),
            ("basket:adxl", "z")
        ]
    );
    assert_eq!(read_register(&mut m.bus, &[0], 1), [0xe5]);
    tx(&mut m.bus, &[0x2d, 8]);
    tx(&mut m.bus, &[0x31, 8]);
    for (c, v) in [("x", 1.), ("y", -0.5), ("z", 0.25)] {
        m.set_input_on("basket:adxl", c, v).unwrap();
    }
    assert_eq!(
        read_register(&mut m.bus, &[0x32], 6),
        [0, 1, 0x80, 0xff, 0x40, 0]
    );
    for (c, v) in [("x", -0.25), ("y", 0.5), ("z", -1.)] {
        m.set_input(c, v).unwrap();
    }
    assert_eq!(
        read_register(&mut m.bus, &[0x32], 6),
        [0xc0, 0xff, 0x80, 0, 0, 0xff]
    );
    let events = m.bus.bus_trace.snapshot();
    use labwired_core::bus::bus_trace::{BusPayload, I2cSym};
    let bytes = events
        .iter()
        .filter_map(|e| match e.payload {
            BusPayload::I2c { kind, byte, ack } => {
                assert!(ack);
                Some((kind, byte))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(bytes.windows(4).any(|w| w
        == [
            (I2cSym::AddrWrite, 0xa6),
            (I2cSym::Data, 0),
            (I2cSym::AddrRead, 0xa7),
            (I2cSym::Data, 0xe5)
        ]));
    assert!(bytes
        .windows(2)
        .any(|w| w == [(I2cSym::AddrWrite, 0xa6), (I2cSym::Data, 0x32)]));
    assert_eq!(read_register(&mut m.bus, &[0x2d], 1), [8]);
    assert_eq!(read_register(&mut m.bus, &[0x31], 1), [8]);
}
#[test]
fn absent_and_static_unpowered_are_only_nak_without_any_inner_calls_and_timeout_stop_works() {
    for off in [false, true] {
        let mut b = bus();
        let (s, p, _) = counted();
        if off {
            b.attach_i2c_slave("i2c3", Box::new(UnpoweredI2cDevice::new(s)))
                .unwrap();
        }
        begin(&mut b, 0);
        fifo(&mut b, &[0xa6, 0x2d, 8]);
        put(&mut b, 0x10, 3);
        assert_eq!(get(&mut b, 0x1c), 0x40);
        assert!(p.lock().unwrap().calls.is_empty());
        put(&mut b, 0x1c, 0x40);
        put(&mut b, 0x18, 0);
        put(&mut b, 0, EN | STOP);
        assert_eq!(get(&mut b, 0x1c), 0x20);
        assert_eq!(controller(&mut b).irq_line_level(), Some(false));
        put(&mut b, 0x1c, 0x7f);
        put(&mut b, 0, 0);
        put(&mut b, 8, 0xa6 | VALID);
        put(&mut b, 12, VALID | 0x32);
        begin(&mut b, REG_TX);
        put(&mut b, 0x14, 6);
        assert_eq!(get(&mut b, 0x1c), 0x40);
        assert!(p.lock().unwrap().calls.is_empty());
        assert!(b.bus_trace.snapshot().is_empty());
        assert!(controller(&mut b).logs()[0].count_matching("NAK") >= 2);
    }
    let mut b = bus();
    attach_canonical(&mut b, false);
    begin(&mut b, 0);
    fifo(&mut b, &[0xa6, 0x2d, 8]);
    put(&mut b, 0x10, 3);
    assert_eq!(get(&mut b, 0x1c), 0x40);
    assert!(b.bus_trace.snapshot().is_empty());
}
#[test]
fn claims_are_rechecked_on_continuation_and_stop_after_cut_does_not_touch_stale_target() {
    let mut b = bus();
    let (s, p, on) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    begin(&mut b, 0);
    fifo(&mut b, &[0xa6, 1]);
    put(&mut b, 0x10, 2);
    put(&mut b, 0x1c, 4);
    let before = p.lock().unwrap().calls.clone();
    on.store(false, Ordering::Relaxed);
    fifo(&mut b, &[2]);
    put(&mut b, 0x10, 1);
    assert_eq!(get(&mut b, 0x1c), 0x40);
    put(&mut b, 0x1c, 0x40);
    stop(&mut b);
    assert_eq!(p.lock().unwrap().calls, before);
}
#[test]
fn stop_requests_once_and_en_disable_restart_and_full_reset_preserve_sensor_state() {
    let mut b = bus();
    let (s, p, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    begin(&mut b, 0);
    fifo(&mut b, &[0xa6, 1]);
    put(&mut b, 0x10, 2);
    put(&mut b, 0x1c, 4);
    put(&mut b, 0, 0x4400);
    assert!(!p.lock().unwrap().calls.contains(&Call::Stop));
    begin(&mut b, 0);
    fifo(&mut b, &[0xa6, 2]);
    put(&mut b, 0x10, 2);
    put(&mut b, 0x1c, 4);
    let c = get(&mut b, 0);
    put(&mut b, 0, c | STOP);
    put(&mut b, 0x1c, 0x20);
    put(&mut b, 0, c | STOP | LASTACK);
    assert_eq!(get(&mut b, 0x1c), 0);
    assert_eq!(
        p.lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| **c == Call::Stop)
            .count(),
        1
    );
    controller(&mut b).reset();
    for o in [0, 4, 8, 12, 16, 20, 24, 28, 32, 0x100, 0x200] {
        assert_eq!(get(&mut b, o), 0);
    }
    assert_eq!(controller(&mut b).irq_line_level(), Some(false));
    let mut b = bus();
    attach_canonical(&mut b, true);
    tx(&mut b, &[0x31, 0x0b]);
    controller(&mut b).reset();
    assert_eq!(read_register(&mut b, &[0x31], 1), [0x0b]);
}
#[test]
fn invalid_width_alignment_count_mode_and_addresses_are_atomic_errors() {
    let mut b = bus();
    let (s, p, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    begin(&mut b, 0);
    let before = controller(&mut b).snapshot();
    assert!(b.write_u8(BASE, 1).is_err());
    assert!(b.write_u16(BASE, 1).is_err());
    assert!(b.read_u8(BASE).is_err());
    assert!(b.read_u16(BASE).is_err());
    assert!(b.write_u32(BASE + 1, 1).is_err());
    assert!(b.read_u32(BASE + 1).is_err());
    for (o, v) in [
        (0x10, 33),
        (0x14, u32::MAX),
        (0x20, 1),
        (0x200, 1),
        (0x21c, 1),
        (0, EN | 6),
        (0x1000, 1),
        (0x120, 1),
        (0x220, 1),
        (8, (1 << 25) | 0xa6),
        (12, 1 << 27),
    ] {
        assert!(controller(&mut b).write_u32(o, v).is_err(), "{o:x}={v:x}");
    }
    assert_eq!(controller(&mut b).snapshot(), before);
    assert!(p.lock().unwrap().calls.is_empty());
    // ACTACK=0 is deliberately bounded like ACTACK=1; it never fakes completion.
    put(&mut b, 0, EN);
    fifo(&mut b, &[0x80]);
    put(&mut b, 0x10, 1);
    assert_eq!(get(&mut b, 0x1c), 0x40);
}
#[test]
fn zero_counts_are_bounded_no_target_calls_and_report_completed_zero() {
    let mut b = bus();
    let (s, p, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    begin(&mut b, 0);
    put(&mut b, 0x10, 0);
    assert!(p.lock().unwrap().calls.is_empty());
    assert_eq!(get(&mut b, 0x20), 0);
    assert_eq!(get(&mut b, 0x1c), 4);
}
#[test]
fn peek_inspect_and_mmio_read_repeats_leave_rx_pending_and_time_unchanged() {
    let mut b = bus();
    let (s, p, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    put(&mut b, 8, VALID | 0xa7);
    begin(&mut b, REG_TX);
    put(&mut b, 0x14, 6);
    let before = controller(&mut b).snapshot();
    let calls = p.lock().unwrap().calls.clone();
    for _ in 0..3 {
        let c = controller(&mut b);
        assert_eq!(c.peek(0x200), Some(0));
        assert_eq!(c.peek(0x201), Some(1));
        assert!(c.peek(0x1000).is_none());
        let pi = c.inspect(BASE, "i2c3", &Default::default());
        assert_eq!(pi.kind, "i2c");
        assert_eq!(c.read_u32(0x200).unwrap(), 0x03020100);
    }
    assert_eq!(controller(&mut b).snapshot(), before);
    assert_eq!(p.lock().unwrap().calls, calls);
}
struct Nested {
    child: GenericI2cDevice,
}
impl I2cDevice for Nested {
    fn address(&self) -> u8 {
        0x70
    }
    fn read(&mut self) -> u8 {
        0
    }
    fn write(&mut self, _: u8) {}
    fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn labwired_core::sim_input::SimInput) -> bool,
    ) -> bool {
        self.child.for_each_sim_input(f)
    }
}
#[test]
fn nested_stimulus_logs_inspect_and_central_time_are_forwarded_once() {
    use labwired_core::sim_input::SimInput;
    let mut b = bus();
    let (s, p, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    let mut child = GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("adxl345").unwrap(),
        0x53,
    )
    .unwrap();
    child.set_component_id("nested:adxl".into());
    b.attach_i2c_slave("i2c3", Box::new(Nested { child }))
        .unwrap();
    let mut m = Machine::new(labwired_core::cpu::cortex_m::CortexM::new(), b);
    m.set_input_on("nested:adxl", "x", 1.).unwrap();
    assert_eq!(m.list_inputs().len(), 3);
    let c = controller(&mut m.bus);
    assert!(c.drives_central_device_time());
    c.advance_attached_device_time_us(7);
    c.advance_attached_device_time_us(3);
    c.tick_elapsed(240);
    assert_eq!(p.lock().unwrap().calls, [Call::Time(7), Call::Time(3)]);
    assert!(c.logs().iter().any(|l| l.name == "probe"));
    let mut addresses = vec![];
    c.for_each_attached_device(&mut |d| addresses.push(d.address.unwrap()));
    assert_eq!(addresses, [0x53, 0x70]);
}

#[test]
fn power_claim_is_rechecked_between_select_start_prefix_and_each_read() {
    for cut in [
        Call::Select(0x53),
        Call::Start,
        Call::Write(0x32),
        Call::Read(0),
    ] {
        let mut b = bus();
        let (s, p, _) = counted();
        p.lock().unwrap().cut_after = Some(cut.clone());
        b.attach_i2c_slave("i2c3", s).unwrap();
        put(&mut b, 8, 0xa6 | VALID);
        put(&mut b, 12, 0x32 | VALID);
        begin(&mut b, REG_TX);
        put(&mut b, 0x14, 6);
        assert_eq!(get(&mut b, 0x1c), 0x40);
        let calls = p.lock().unwrap().calls.clone();
        assert_eq!(calls.last(), Some(&cut));
        put(&mut b, 0x1c, 0x40);
        stop(&mut b);
        assert_eq!(p.lock().unwrap().calls, calls);
    }
}
#[test]
fn completed_read_cut_continuation_does_not_call_inner_or_complete() {
    let mut b = bus();
    let (s, p, on) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    put(&mut b, 8, 0xa7 | VALID);
    begin(&mut b, REG_TX);
    put(&mut b, 0x14, 32);
    put(&mut b, 0x1c, 8);
    let calls = p.lock().unwrap().calls.clone();
    on.store(false, Ordering::Relaxed);
    put(&mut b, 0, EN | RX | ACTACK | LASTACK);
    put(&mut b, 0x14, 1);
    assert_eq!(get(&mut b, 0x1c), 0x40);
    assert_eq!(p.lock().unwrap().calls, calls);
}
#[test]
fn slave_artifact_and_pin_queue_forward_through_controller_inspection() {
    let mut b = bus();
    let (s, _, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    let c = controller(&mut b);
    let opts = labwired_core::inspect::InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    let pi = c.inspect(BASE, "i2c3", &opts);
    assert_eq!(pi.artifacts[0].bytes, Some(vec![0x53]));
    let mut artifacts = vec![];
    c.for_each_attached_device(&mut |d| {
        artifacts.extend(d.evidence.unwrap().artifacts("probe", &opts))
    });
    assert_eq!(artifacts.len(), 1);
    let mut pins = vec![];
    c.drain_attached_pin_drives(&mut pins);
    assert_eq!(pins, [("probe".into(), "irq".into(), true)]);
    pins.clear();
    c.drain_attached_pin_drives(&mut pins);
    assert!(pins.is_empty());
}

#[test]
fn machine_cycle_derived_device_time_reaches_slave_exactly_once() {
    use labwired_core::{AdvanceRequest, Cpu};
    // A closed RV1103 bus permits genuine architectural idle; the generic
    // SystemBus::new test fixture also contains unrelated STM32 legacy models.
    let mut b = SystemBus::empty();
    b.cpu_hz = 24_000_000;
    b.ram = labwired_core::memory::LinearMemory::new(0x10000, 0);
    b.configure_armv7_rv1103().unwrap();
    b.configure_armv7_rv1103_i2c3().unwrap();
    let (s, p, _) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    b.ram.data[..2].copy_from_slice(&0xe7feu16.to_le_bytes());
    let mut cpu = labwired_core::cpu::cortex_m::CortexM::new();
    cpu.set_pc(b.ram.base_addr as u32);
    let mut m = Machine::new(cpu, b);
    m.config.peripheral_tick_interval = 24;
    m.advance(AdvanceRequest::run(Some(1))).unwrap(); // time drive anchors its initial slice
    p.lock().unwrap().calls.clear();
    let start_us = m.total_cycles / 24;
    m.advance(AdvanceRequest::run(Some(240))).unwrap();
    let calls = p.lock().unwrap().calls.clone();
    let elapsed = m.total_cycles / 24 - start_us;
    assert!(elapsed > 0);
    assert_eq!(
        calls
            .iter()
            .filter_map(|c| if let Call::Time(us) = c {
                Some(us)
            } else {
                None
            })
            .sum::<u64>(),
        elapsed
    );
    assert!(calls.iter().all(|c| matches!(c, Call::Time(_))));

    #[cfg(feature = "event-scheduler")]
    {
        // Real masked WFI, then a masked pending SysTick wakes the CPU without
        // entering a handler. Device time must cover both idle and active work
        // exactly once through the same central clock.
        let code = m.bus.ram.base_addr;
        m.bus.write_u16(code, 0xb672).unwrap(); // CPSID i
        m.bus.write_u16(code + 2, 0xbf30).unwrap(); // WFI
        m.bus.write_u16(code + 4, 0xbf00).unwrap(); // NOP
        m.bus.write_u16(code + 6, 0xe7fe).unwrap(); // B .
        m.cpu.invalidate_code_caches();
        m.cpu.set_pc(code as u32);
        m.config.idle_fast_forward_enabled = true;
        let entering = m.advance(AdvanceRequest::run(Some(2))).unwrap();
        assert_eq!(entering.primary_steps, 2);
        assert_eq!(m.cpu.get_pc(), code as u32 + 4);
        p.lock().unwrap().calls.clear();
        let start_us = m.total_cycles / 24;
        for _ in 0..8 {
            let idle = m.advance(AdvanceRequest::run(Some(4096))).unwrap();
            assert_eq!(idle.primary_steps, 0);
            assert_eq!(idle.idle_cycles, 4096);
        }
        let idle_us = m.total_cycles / 24 - start_us;
        let time_sum = || {
            p.lock()
                .unwrap()
                .calls
                .iter()
                .filter_map(|call| {
                    if let Call::Time(us) = call {
                        Some(*us)
                    } else {
                        None
                    }
                })
                .sum::<u64>()
        };
        assert_eq!(
            time_sum(),
            idle_us,
            "idle time must reach the slave exactly once"
        );
        m.cpu.set_exception_pending(15);
        let wake = m.advance(AdvanceRequest::run(Some(1))).unwrap();
        assert_eq!(wake.primary_steps, 1);
        assert_eq!(wake.idle_cycles, 0);
        assert_eq!(m.cpu.get_pc(), code as u32 + 6);
        let active = m.advance(AdvanceRequest::run(Some(240))).unwrap();
        assert_eq!(active.primary_steps, 240);
        assert_eq!(active.idle_cycles, 0);
        assert_eq!(
            time_sum(),
            m.total_cycles / 24 - start_us,
            "idle-to-active transition must neither lose nor double device time"
        );
        assert!(p
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|c| matches!(c, Call::Time(_))));
    }
}
#[test]
fn manifest_funnel_rejects_unknown_missing_controller_duplicate_ids_and_repeated_attach() {
    let mut b = bus();
    let mut m = canonical_manifest(true);
    m.external_devices[0].r#type = "unknown_rv_part".into();
    assert!(b.attach_manifest_external_devices(&m).is_err());
    assert!(b.list_inputs().is_empty());
    let mut b = bus();
    let mut m = canonical_manifest(true);
    m.external_devices[0].connection = "missing".into();
    assert!(b.attach_manifest_external_devices(&m).is_err());
    assert!(b.list_inputs().is_empty());
    let mut b = bus();
    let mut m = canonical_manifest(true);
    m.external_devices.push(m.external_devices[0].clone());
    assert!(b.attach_manifest_external_devices(&m).is_err());
    assert!(b.list_inputs().is_empty());
    assert!(b.external_device_decls.is_empty());
    let mut b = bus();
    let m = canonical_manifest(true);
    b.attach_manifest_external_devices(&m).unwrap();
    let count = b.list_inputs().len();
    assert!(b.attach_manifest_external_devices(&m).is_err());
    assert_eq!(b.list_inputs().len(), count);
}
#[test]
fn canonical_manifest_inspection_joins_real_type_id_and_address() {
    let mut b = bus();
    attach_canonical(&mut b, true);
    let m = Machine::new(labwired_core::cpu::cortex_m::CortexM::new(), b);
    let inspected = m.inspect(
        None,
        &labwired_core::inspect::InspectOpts {
            peripheral: Some("i2c3".into()),
            include_bytes: false,
        },
    );
    assert_eq!(inspected.devices.len(), 1);
    let d = &inspected.devices[0];
    assert_eq!(d.id, "basket:adxl");
    assert_eq!(d.device_type.as_deref(), Some("adxl345"));
    assert!(d.declared);
    assert_eq!(d.attachment.address, Some(0x53));
}
#[test]
fn install_refuses_cpu_memory_mappings_and_duplicate_peripheral_name() {
    for extra in [false, true] {
        let mut b = SystemBus::new();
        b.cpu_hz = 24_000_000;
        b.configure_armv7_rv1103().unwrap();
        let mem = labwired_core::memory::LinearMemory::new(0x100, BASE + 0x100);
        if extra {
            b.extra_mem.push(mem)
        } else {
            b.ram = mem
        }
        let n = b.peripherals.len();
        assert!(b.configure_armv7_rv1103_i2c3().is_err());
        assert_eq!(b.peripherals.len(), n);
    }
    let mut b = SystemBus::new();
    b.cpu_hz = 24_000_000;
    b.configure_armv7_rv1103().unwrap();
    let g = b.irq_fabric.armv7.as_ref().unwrap().gic.clone();
    b.add_peripheral(
        "i2c3",
        BASE + 0x10000,
        0x100,
        None,
        Box::new(Rk3xI2c::new(g, 53)),
    );
    assert!(b.configure_armv7_rv1103_i2c3().is_err());
}
#[test]
fn condition_requests_are_not_replayed_by_unrelated_con_writes_and_logs_are_bounded() {
    let mut b = bus();
    put(&mut b, 0, EN | START | ACTACK);
    put(&mut b, 0x1c, 0x10);
    put(&mut b, 0, EN | START | ACTACK | LASTACK);
    assert_eq!(get(&mut b, 0x1c), 0);
    for _ in 0..300 {
        put(&mut b, 0, EN | ACTACK);
        put(&mut b, 0, EN | START | ACTACK);
        put(&mut b, 0x1c, 0x10);
        put(&mut b, 0, EN | STOP | ACTACK);
        put(&mut b, 0x1c, 0x20);
    }
    let logs = controller(&mut b).logs();
    assert!(logs[0].entries.len() <= 256);
    assert!(logs[0].count_matching("STOP") > 0);
}
#[test]
fn failed_chunk_remains_nak_until_restart_even_if_slave_returns() {
    let mut b = bus();
    let (s, p, on) = counted();
    b.attach_i2c_slave("i2c3", s).unwrap();
    begin(&mut b, 0);
    on.store(false, Ordering::Relaxed);
    fifo(&mut b, &[0xa6]);
    put(&mut b, 0x10, 1);
    assert_eq!(get(&mut b, 0x1c), 0x40);
    put(&mut b, 0x1c, 0x40);
    on.store(true, Ordering::Relaxed);
    put(&mut b, 0x10, 1);
    assert_eq!(get(&mut b, 0x1c), 0x40);
    assert!(p.lock().unwrap().calls.is_empty());
    put(&mut b, 0, 0);
    begin(&mut b, 0);
    put(&mut b, 0x10, 1);
    assert_eq!(get(&mut b, 0x1c), 4);
    assert_eq!(p.lock().unwrap().calls, [Call::Select(0x53), Call::Start]);
}
