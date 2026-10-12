#![cfg(feature = "native")]
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_BUS: AtomicU64 = AtomicU64::new(1);
use labwired_arm_system::{ArmCpu, PhysicalBus};
struct Bus {
    identity: u64,
    cp_value: Option<u64>,
    rendezvous: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    ram: Vec<u8>,
    io: u64,
    accesses: Vec<(bool, u64, u32, u64)>,
    fail: bool,
    panic: bool,
    cp: Vec<(u32, bool, u64)>,
}
impl Bus {
    fn new() -> Self {
        Self {
            identity: NEXT_BUS.fetch_add(1, Ordering::Relaxed),
            cp_value: None,
            rendezvous: None,
            ram: vec![0; 0x10000],
            io: 0,
            accesses: vec![],
            fail: false,
            panic: false,
            cp: vec![],
        }
    }
    fn words(&mut self, a: usize, ws: &[u32]) {
        for (i, w) in ws.iter().enumerate() {
            self.ram[a + i * 4..a + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
}
impl PhysicalBus for Bus {
    fn code_identity(&self) -> u64 {
        self.identity
    }
    fn read(&mut self, a: u64, w: u32, s: u64) -> Result<u64, String> {
        if self.panic {
            panic!("bus panic")
        }
        if self.fail {
            return Err("bus failure".into());
        }
        self.accesses.push((false, a, w, s));
        if a == 0x100000 {
            if let Some((tx, rx)) = self.rendezvous.take() {
                tx.send(()).map_err(|e| e.to_string())?;
                rx.recv_timeout(std::time::Duration::from_secs(5))
                    .map_err(|e| e.to_string())?;
            }
            return Ok(self.io);
        }
        let p = a as usize;
        let b = self.ram.get(p..p + w as usize).ok_or("unmapped read")?;
        let mut bytes = [0; 8];
        bytes[..b.len()].copy_from_slice(b);
        Ok(u64::from_le_bytes(bytes))
    }
    fn write(&mut self, a: u64, w: u32, v: u64, s: u64) -> Result<(), String> {
        self.accesses.push((true, a, w, s));
        if a == 0x100000 {
            self.io = v;
            return Ok(());
        }
        let p = a as usize;
        self.ram
            .get_mut(p..p + w as usize)
            .ok_or("unmapped write")?
            .copy_from_slice(&v.to_le_bytes()[..w as usize]);
        Ok(())
    }
    fn cp15_read(&mut self, i: u32, w: bool) -> Result<u64, String> {
        self.cp.push((i, w, 0));
        self.cp_value.ok_or("unknown external CP15".into())
    }
    fn cp15_write(&mut self, i: u32, w: bool, v: u64) -> Result<(), String> {
        self.cp.push((i, w, v));
        self.cp_value
            .map(|_| ())
            .ok_or("unknown external CP15".into())
    }
}
fn cpu(b: &mut Bus, code: &[u32]) -> ArmCpu {
    b.words(0x100, code);
    let mut c = ArmCpu::new().unwrap();
    c.executable_range(0, 0x10000).unwrap();
    c.set_register(15, 0x100).unwrap();
    c
}
#[test]
fn arithmetic_and_exact_width_mmio() {
    let mut b = Bus::new();
    let mut c = cpu(
        &mut b,
        &[
            0xe3a0002a, 0xe2800001, 0xe5c10000, 0xe5d12000, 0xe1c100b0, 0xe1d130b0, 0xe5810000,
            0xe5914000,
        ],
    );
    c.set_register(1, 0x100000).unwrap();
    assert_eq!(c.execute(&mut b, 8).unwrap().retired, 8);
    assert_eq!(c.register(0).unwrap(), 43);
    for r in [2, 3, 4] {
        assert_eq!(c.register(r).unwrap(), 43)
    }
    for w in [1, 2, 4] {
        assert!(b
            .accesses
            .iter()
            .any(|x| x.0 && x.1 == 0x100000 && x.2 == w));
        assert!(b
            .accesses
            .iter()
            .any(|x| !x.0 && x.1 == 0x100000 && x.2 == w));
    }
}
#[test]
fn mmu_short_descriptor_and_guest_abort() {
    let mut b = Bus::new();
    b.words(0x300, &[42]);
    b.words(0x4000, &[0xc02]);
    b.words(0x6000, &[0xc02]);
    let mut c = cpu(
        &mut b,
        &[
            0xee020f10, 0xee031f10, 0xee112f10, 0xe3822001, 0xee012f10, 0xe5934000,
        ],
    );
    c.set_register(0, 0x4000).unwrap();
    c.set_register(1, 3).unwrap();
    c.set_register(3, 0x80000300).unwrap();
    assert_eq!(c.execute(&mut b, 6).unwrap().retired, 6);
    assert_eq!(c.register(4).unwrap(), 42);
    assert!(b.accesses.iter().any(|x| x.1 == 0x300));
    assert!(!b.accesses.iter().any(|x| x.1 == 0x80000300));
    b.words(0x118, &[0xe5956000]);
    b.words(0x10, &[0xe3a07063]);
    c.invalidate_code().unwrap();
    c.set_register(5, 0x90000000).unwrap();
    c.execute(&mut b, 2).unwrap();
    assert_eq!(c.register(16).unwrap() & 31, 0x17);
    assert_eq!(c.register(7).unwrap(), 99);
}
#[test]
fn svc_irq_and_deassertion() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[0xef000000]);
    b.words(8, &[0xe3a05007]);
    b.words(0x18, &[0xe3a0600d]);
    c.execute(&mut b, 2).unwrap();
    assert_eq!(c.register(5).unwrap(), 7);
    assert_eq!(c.register(16).unwrap() & 31, 0x13);
    c.set_register(16, 0x13).unwrap();
    c.interrupt_levels(true, false).unwrap();
    let e = c.execute(&mut b, 1).unwrap();
    assert_eq!(e.retired, 0);
    assert!(!e.waiting);
    assert_eq!(c.register(15).unwrap(), 0x18);
    assert_eq!(c.register(16).unwrap() & 31, 0x12);
    c.interrupt_levels(false, false).unwrap();
    assert_eq!(c.execute(&mut b, 1).unwrap().retired, 1);
    assert_eq!(c.register(6).unwrap(), 13);
}
#[test]
fn thumb2_and_waiting() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[]);
    b.ram[0x100..0x10a]
        .copy_from_slice(&[0x40, 0xf2, 0x2a, 0x00, 0x01, 0x30, 0x30, 0xbf, 0x00, 0xbf]);
    c.set_register(16, 0x33).unwrap();
    assert_eq!(c.execute(&mut b, 3).unwrap().retired, 3);
    assert_eq!(c.register(0).unwrap(), 43);
    assert!(c.execute(&mut b, 1).unwrap().waiting);
}
#[test]
fn failures_and_panics_poison_until_reset() {
    for panic in [false, true] {
        let mut b = Bus::new();
        let mut c = cpu(&mut b, &[0xe3a00001]);
        b.panic = panic;
        b.fail = !panic;
        assert!(c.execute(&mut b, 1).is_err());
        b.panic = false;
        b.fail = false;
        assert!(c.execute(&mut b, 1).is_err());
        assert!(c.register(0).is_err());
        c.reset().unwrap();
        c.set_register(15, 0x100).unwrap();
        assert_eq!(c.execute(&mut b, 1).unwrap().retired, 1);
    }
}
#[test]
fn guest_and_host_code_modification_and_bus_swap() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[0xe3a00001]);
    c.execute(&mut b, 1).unwrap();
    b.words(0x100, &[0xe3a00002]);
    c.invalidate_code().unwrap();
    c.set_register(15, 0x100).unwrap();
    c.execute(&mut b, 1).unwrap();
    assert_eq!(c.register(0).unwrap(), 2);
    let mut other = Bus::new();
    other.words(0x100, &[0xe3a00003]);
    c.set_register(15, 0x100).unwrap();
    c.execute(&mut other, 1).unwrap();
    assert_eq!(c.register(0).unwrap(), 3);
    other.words(0x200, &[0xe5812000, 0xeaffffbd]);
    c.set_register(15, 0x200).unwrap();
    c.set_register(1, 0x100).unwrap();
    c.set_register(2, 0xe3a00004).unwrap();
    c.execute(&mut other, 3).unwrap();
    assert_eq!(c.register(0).unwrap(), 4);
}
#[test]
fn isolated_instances_drop_and_threads() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[0xe2800001]);
    let mut b2 = Bus::new();
    let mut c2 = cpu(&mut b2, &[0xe2800002]);
    for _ in 0..3 {
        c.set_register(15, 0x100).unwrap();
        c.execute(&mut b, 1).unwrap();
        c2.set_register(15, 0x100).unwrap();
        c2.execute(&mut b2, 1).unwrap();
    }
    assert_eq!(c.register(0).unwrap(), 3);
    assert_eq!(c2.register(0).unwrap(), 6);
    drop(c);
    // Both CPUs must enter their physical MMIO callback before either returns.
    let (tx_a, rx_a) = std::sync::mpsc::channel();
    let (tx_b, rx_b) = std::sync::mpsc::channel();
    b2.words(0x100, &[0xe5910000]);
    b2.io = 8;
    b2.rendezvous = Some((tx_a, rx_b));
    c2.invalidate_code().unwrap();
    c2.set_register(1, 0x100000).unwrap();
    let mut b3 = Bus::new();
    let mut c3 = cpu(&mut b3, &[0xe5910000]);
    b3.io = 9;
    b3.rendezvous = Some((tx_b, rx_a));
    c3.set_register(1, 0x100000).unwrap();
    let h = std::thread::spawn(move || {
        c2.set_register(15, 0x100).unwrap();
        assert_eq!(c2.execute(&mut b2, 1).unwrap().retired, 1);
        assert_eq!(c2.register(0).unwrap(), 8)
    });
    assert_eq!(c3.execute(&mut b3, 1).unwrap().retired, 1);
    assert_eq!(c3.register(0).unwrap(), 9);
    h.join().unwrap();
}

#[test]
fn bus_reallocated_in_same_storage_has_fresh_code() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[0xe3a00001]);
    c.execute(&mut b, 1).unwrap();
    b = Bus::new();
    b.words(0x100, &[0xe3a00007]);
    c.set_register(15, 0x100).unwrap();
    c.execute(&mut b, 1).unwrap();
    assert_eq!(c.register(0).unwrap(), 7);
}
#[test]
fn stable_bus_keeps_translated_code() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[0xe3a00001]);
    c.execute(&mut b, 1).unwrap();
    b.accesses.clear();
    c.set_register(15, 0x100).unwrap();
    c.execute(&mut b, 1).unwrap();
    assert!(
        !b.accesses.iter().any(|x| x.1 == 0x100),
        "stable bus must retain its translated code"
    );
}
#[test]
fn cp15_external_32_and_64_forwarding_and_unknown_error() {
    let mut b = Bus::new();
    b.cp_value = Some(0x1122334455667788);
    let mut c = cpu(&mut b, &[0xee1e0f10, 0xee0e0f10, 0xec510f0e, 0xec410f0e]);
    assert_eq!(c.execute(&mut b, 4).unwrap().retired, 4);
    assert_eq!(
        b.cp,
        vec![
            (0xe0000, false, 0),
            (0xe0000, false, 0x55667788),
            (0xe, true, 0),
            (0xe, true, 0x1122334455667788)
        ]
    );
    assert!(!b.cp[0].1);
    assert!(!b.cp[1].1);
    assert!(b.cp[2].1);
    assert!(b.cp[3].1);
    assert_eq!(b.cp[1].2, 0x55667788);
    assert_eq!(b.cp[3].2, 0x1122334455667788);
    assert_eq!(c.register(0).unwrap(), 0x55667788);
    assert_eq!(c.register(1).unwrap(), 0x11223344);
    c.reset().unwrap();
    c.set_register(15, 0x100).unwrap();
    b.cp_value = None;
    assert!(c
        .execute(&mut b, 1)
        .unwrap_err()
        .contains("unknown external CP15"));
}
#[test]
fn eight_byte_vfp_physical_transactions() {
    let mut b = Bus::new();
    b.words(0x300, &[0x55667788, 0x11223344]);
    let mut c = cpu(
        &mut b,
        &[
            0xee110f50, 0xe380060f, 0xee010f50, 0xe3a00101, 0xeee80a10, 0xed910b00, 0xed820b00,
        ],
    );
    c.set_register(1, 0x300).unwrap();
    c.set_register(2, 0x308).unwrap();
    assert_eq!(c.execute(&mut b, 7).unwrap().retired, 7);
    assert_eq!(&b.ram[0x308..0x310], &0x1122334455667788u64.to_le_bytes());
    assert!(b.accesses.iter().any(|x| !x.0 && x.1 == 0x300 && x.2 == 8));
    assert!(b.accesses.iter().any(|x| x.0 && x.1 == 0x308 && x.2 == 8));
    b.io = 0x1122334455667788;
    b.accesses.clear();
    c.set_register(15, 0x100).unwrap();
    c.set_register(1, 0x100000).unwrap();
    c.set_register(2, 0x100000).unwrap();
    assert_eq!(c.execute(&mut b, 7).unwrap().retired, 7);
    assert_eq!(b.io, 0x1122334455667788);
    assert!(b
        .accesses
        .iter()
        .any(|x| !x.0 && x.1 == 0x100000 && x.2 == 8));
    assert!(b
        .accesses
        .iter()
        .any(|x| x.0 && x.1 == 0x100000 && x.2 == 8));
}

#[test]
fn cp15_nonzero_op_fields_are_canonical_keys() {
    let mut b = Bus::new();
    b.cp_value = Some(0x123456789abcdef0);
    let mut c = cpu(&mut b, &[0xee7e0f55, 0xee6e0f55, 0xec510f7e, 0xec410f7e]);
    c.execute(&mut b, 4).unwrap();
    assert_eq!(
        b.cp,
        vec![
            (0x6e0045, false, 0),
            (0x6e0045, false, 0x9abcdef0),
            (0x7e, true, 0),
            (0x7e, true, 0x123456789abcdef0)
        ]
    );
}
#[test]
fn executable_range_crosses_native_page_boundary() {
    let mut b = Bus::new();
    b.words(0x400, &[0xe3a0002a]);
    let mut c = ArmCpu::new().unwrap();
    c.executable_range(0x3ff, 5).unwrap();
    c.set_register(15, 0x400).unwrap();
    assert_eq!(c.execute(&mut b, 1).unwrap().retired, 1);
    assert_eq!(c.register(0).unwrap(), 42);
    c.reset().unwrap();
    c.set_register(15, 0x400).unwrap();
    assert_eq!(c.execute(&mut b, 1).unwrap().retired, 1);
}
#[test]
fn executable_ranges_reject_overflow_and_empty() {
    let mut c = ArmCpu::new().unwrap();
    for (s, n) in [(0, 0), (u64::MAX, 2), (1 << 32, 1), (0xffff_ffff, 2)] {
        assert!(c.executable_range(s, n).is_err());
    }
    c.executable_range(0xffff_ffff, 1).unwrap();
}

#[test]
fn execute_rejects_zero_budget_without_poisoning() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[0xe3a00001]);
    assert!(c.execute(&mut b, 0).is_err());
    assert_eq!(c.execute(&mut b, 1).unwrap().retired, 1);
}
#[test]
fn lpae_guest_enable_is_refused_before_following_access() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[0xee020f50, 0xe5821000]);
    c.set_register(0, 0x80000000).unwrap();
    c.set_register(1, 0x55).unwrap();
    c.set_register(2, 0x100000).unwrap();
    let error = c.execute(&mut b, 2).unwrap_err();
    assert!(error.contains("LPAE"), "{error}");
    assert_eq!(b.io, 0);
    assert!(!b.accesses.iter().any(|x| x.0 && x.1 == 0x100000));
    assert!(c.register(0).is_err());
    assert!(c.execute(&mut b, 1).is_err());
    c.reset().unwrap();
    c.set_register(15, 0x104).unwrap();
    c.set_register(1, 0x55).unwrap();
    c.set_register(2, 0x100000).unwrap();
    assert_eq!(c.execute(&mut b, 1).unwrap().retired, 1);
    assert_eq!(b.io, 0x55);
}

#[test]
fn exclusive_arm_instructions_are_safe_and_isolated() {
    if std::env::var_os("LABWIRED_ARM_EXCLUSIVE_PROBE").is_some() {
        let instructions = [0xe1910f9f, 0xe1813f92, 0xe1915f9f, 0xf57ff01f, 0xe1814f92];
        let mut a = Bus::new();
        a.words(0x300, &[10]);
        let mut ca = cpu(&mut a, &instructions);
        let mut b = Bus::new();
        b.words(0x300, &[20]);
        let mut cb = cpu(&mut b, &instructions);
        for (c, value) in [(&mut ca, 42), (&mut cb, 99)] {
            c.set_register(1, 0x300).unwrap();
            c.set_register(2, value).unwrap();
        }
        assert_eq!(ca.execute(&mut a, 1).unwrap().retired, 1);
        assert_eq!(ca.register(0).unwrap(), 10);
        assert_eq!(cb.execute(&mut b, 1).unwrap().retired, 1);
        assert_eq!(cb.register(0).unwrap(), 20);
        assert_eq!(ca.execute(&mut a, 1).unwrap().retired, 1);
        assert_eq!(ca.register(3).unwrap(), 0);
        assert_eq!(&a.ram[0x300..0x304], &42u32.to_le_bytes());
        drop(ca);
        // Keep B's reservation across dropping A and moving the idle CPU thread.
        let (mut cb, mut b) = std::thread::spawn(move || {
            assert_eq!(cb.execute(&mut b, 4).unwrap().retired, 4);
            assert_eq!(cb.register(3).unwrap(), 0);
            assert_eq!(cb.register(5).unwrap(), 99);
            assert_eq!(cb.register(4).unwrap(), 1);
            assert_eq!(&b.ram[0x300..0x304], &99u32.to_le_bytes());
            assert_eq!(
                b.accesses
                    .iter()
                    .filter(|x| x.0 && x.1 == 0x300 && x.2 == 4)
                    .count(),
                1
            );
            (cb, b)
        })
        .join()
        .unwrap();
        // Identical addresses and values in another physical bus must not
        // inherit the reservation owned by the original memory domain.
        cb.set_register(15, 0x100).unwrap();
        cb.execute(&mut b, 1).unwrap();
        let mut replacement = Bus::new();
        replacement.words(0x100, &instructions);
        replacement.words(0x300, &[99]);
        cb.set_register(2, 123).unwrap();
        cb.execute(&mut replacement, 1).unwrap();
        assert_eq!(
            cb.register(3).unwrap(),
            1,
            "reservation must be cleared on physical bus replacement"
        );
        assert_eq!(&replacement.ram[0x300..0x304], &99u32.to_le_bytes());
        assert!(!replacement.accesses.iter().any(|x| x.0 && x.1 == 0x300));
        // Reset must discard an outstanding exclusive monitor reservation.
        cb.set_register(15, 0x100).unwrap();
        cb.execute(&mut b, 1).unwrap();
        cb.reset().unwrap();
        cb.set_register(15, 0x104).unwrap();
        cb.set_register(1, 0x300).unwrap();
        cb.set_register(2, 123).unwrap();
        cb.execute(&mut b, 1).unwrap();
        assert_eq!(cb.register(3).unwrap(), 1);
        assert_eq!(&b.ram[0x300..0x304], &99u32.to_le_bytes());
        return;
    }
    // Run native regressions in a child so a missing native prerequisite gives
    // a useful test failure instead of terminating the entire architecture suite.
    let status = std::process::Command::new("sh")
        .args(["-c", "ulimit -c 0; exec \"$@\"", "--"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "exclusive_arm_instructions_are_safe_and_isolated",
        ])
        .env("LABWIRED_ARM_EXCLUSIVE_PROBE", "1")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "exclusive instruction subprocess exited {status}"
    );
}

#[test]
fn disposing_jit_preserves_a_reallocated_mapping() {
    if std::env::var_os("LABWIRED_ARM_UNMAP_PROBE").is_some() {
        let mut b = Bus::new();
        let mut c = cpu(&mut b, &[0xe3a0002a]);
        c.execute(&mut b, 1).unwrap();
        drop(c);
        let this = libloading::os::unix::Library::this();
        let check =
            unsafe { this.get::<unsafe extern "C" fn() -> i32>(b"lw_check_unmap_replacement\0") }
                .unwrap();
        assert_eq!(
            unsafe { check() },
            0,
            "disposing one JIT must not unmap its address after another owner reallocates it"
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("unmap_probe.c");
    let library = directory.path().join("unmap_probe.so");
    std::fs::write(&source, include_bytes!("unmap_probe.c")).unwrap();
    assert!(std::process::Command::new("cc")
        .args(["-shared", "-fPIC", "-O2", "-o"])
        .arg(&library)
        .arg(&source)
        .status()
        .unwrap()
        .success());
    let status = std::process::Command::new("sh")
        .args(["-c", "ulimit -c 0; exec \"$@\"", "--"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "disposing_jit_preserves_a_reallocated_mapping"])
        .env("LABWIRED_ARM_UNMAP_PROBE", "1")
        .env("LD_PRELOAD", library)
        .status()
        .unwrap();
    assert!(status.success(), "JIT reallocation probe exited {status}");
}

#[test]
fn concurrent_creation_execution_and_disposal() {
    let threads = (0..8)
        .map(|thread| {
            std::thread::spawn(move || {
                for iteration in 0..24 {
                    let value = thread * 24 + iteration + 1;
                    let mut b = Bus::new();
                    let mut c = cpu(&mut b, &[0xe2800001, 0xeafffffd]);
                    c.set_register(0, value).unwrap();
                    assert_eq!(c.execute(&mut b, 200).unwrap().retired, 200);
                    assert_eq!(c.register(0).unwrap(), value + 100);
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn translated_trace_records_real_irq_instruction_and_mmu_fetch() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[0xe3a0002a]);
    b.words(0x18, &[0xe3a0600d]);
    c.set_register(16, 0x13).unwrap();
    c.interrupt_levels(true, false).unwrap();
    let (e, t) = c.execute_traced(&mut b).unwrap();
    assert_eq!(e.retired, 0);
    assert_eq!(t, None);
    c.interrupt_levels(false, false).unwrap();
    let (e, t) = c.execute_traced(&mut b).unwrap();
    assert_eq!(e.retired, 1);
    let t = t.unwrap();
    assert_eq!((t.pc, t.opcode), (0x18, 0xe3a0600d));
    let mut b = Bus::new();
    let mut c = cpu(
        &mut b,
        &[0xee020f10, 0xee031f10, 0xee112f10, 0xe3822001, 0xee012f10],
    );
    b.words(0x300, &[0xe3a0402a]);
    b.words(0x4000, &[0xc02]);
    b.words(0x6000, &[0xc02]);
    c.set_register(0, 0x4000).unwrap();
    c.set_register(1, 3).unwrap();
    c.execute(&mut b, 5).unwrap();
    c.set_register(15, 0x80000300).unwrap();
    let (e, t) = c.execute_traced(&mut b).unwrap();
    assert_eq!(e.retired, 1);
    let t = t.unwrap();
    assert_eq!((t.pc, t.opcode), (0x80000300, 0xe3a0402a));
    assert_eq!(c.register(4).unwrap(), 42);
    assert!(!b.accesses.iter().any(|x| x.1 == 0x80000300));
}

#[test]
fn traced_thumb32_crosses_physical_native_page_boundary() {
    let mut b = Bus::new();
    b.ram[0x3fe..0x402].copy_from_slice(&[0x40, 0xf2, 0x2a, 0x00]);
    let mut c = ArmCpu::new().unwrap();
    c.executable_range(0, 0x10000).unwrap();
    c.set_register(16, 0x33).unwrap();
    c.set_register(15, 0x3fe).unwrap();
    let (e, t) = c.execute_traced(&mut b).unwrap();
    assert_eq!(e.retired, 1);
    let t = t.unwrap();
    assert_eq!((t.pc, t.opcode), (0x3fe, 0x002af240));
    assert_eq!(c.register(0).unwrap(), 42);
    assert!(b.accesses.iter().any(|x| x.1 == 0x400 && x.2 == 2));
}

#[test]
fn arm_and_thumb_self_branches_retire_without_waiting() {
    for thumb in [false, true] {
        let mut b = Bus::new();
        let mut c = cpu(&mut b, &[if thumb { 0xe7fee7fe } else { 0xeafffffe }]);
        if thumb {
            c.set_register(16, 0x33).unwrap();
        }
        for _ in 0..8 {
            let e = c.execute(&mut b, 1).unwrap();
            assert_eq!(e.retired, 1);
            assert!(!e.waiting);
            assert_eq!(c.register(15).unwrap(), 0x100);
        }
        assert_eq!(c.execute(&mut b, 8).unwrap().retired, 8);
        assert!(!c.execute(&mut b, 1).unwrap().waiting);
        // A real WFI retains architectural wait after fixing only branch loops.
        b.words(0x100, &[if thumb { 0xbf00bf30 } else { 0xe320f003 }]);
        c.invalidate_code().unwrap();
        assert_eq!(c.execute(&mut b, 1).unwrap().retired, 1);
        let e = c.execute(&mut b, 1).unwrap();
        assert_eq!(e.retired, 0);
        assert!(e.waiting);
    }
}

#[test]
fn unmapped_abort_vector_fails_without_native_livelock() {
    if std::env::var_os("LABWIRED_NATIVE_VECTOR_PROBE").is_some() {
        let mut b = Bus::new();
        let mut c = cpu(
            &mut b,
            &[0xee020f10, 0xee031f10, 0xee112f10, 0xe3822001, 0xee012f10],
        );
        c.set_register(0, 0x4000).unwrap();
        c.set_register(1, 3).unwrap();
        // Zero table entries leave both the running VA and abort vector unmapped.
        assert_eq!(c.execute(&mut b, 5).unwrap().retired, 5);
        let error = c.execute(&mut b, 1).unwrap_err();
        assert!(error.contains("without retirement"), "{error}");
        for field in [
            "pc=0xc",
            "first_pc=0x114",
            "first_ifar=0x114",
            "first_ttbr0=0x4000",
            "ifsr=",
            "dfar=",
            "dfsr=",
            "sctlr=",
            "ttbr1=",
            "dacr=",
        ] {
            assert!(error.contains(field), "missing {field}: {error}");
        }
        assert!(c.register(0).is_err());
        c.reset().unwrap();
        assert_eq!(c.register(0).unwrap(), 0);
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unmapped_abort_vector_fails_without_native_livelock",
        ])
        .env("LABWIRED_NATIVE_VECTOR_PROBE", "1")
        .spawn()
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "native vector probe exited {status}");
            break;
        }
        if std::time::Instant::now() >= until {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native execution hung on unmapped abort vector");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
#[test]
fn repeated_svc_retirement_does_not_trigger_exception_livelock_guard() {
    let mut b = Bus::new();
    let mut c = cpu(&mut b, &[0xef000000]);
    b.words(8, &[0xef000000]);
    assert_eq!(c.execute(&mut b, 64).unwrap().retired, 64);
    assert_eq!(c.register(15).unwrap(), 8);
    assert!(!c.execute(&mut b, 1).unwrap().waiting);
}

#[test]
fn translation_cache_pressure_preserves_jit_prologue_and_retirement() {
    if std::env::var_os("LABWIRED_NATIVE_CACHE_PROBE").is_some() {
        let instructions = 200_000u32;
        let mut b = Bus::new();
        b.ram.resize(0x100000, 0);
        b.words(0x100, &vec![0xe2800001; instructions as usize]);
        let mut c = ArmCpu::new().unwrap();
        c.executable_range(0, b.ram.len() as u64).unwrap();
        c.set_register(15, 0x100).unwrap();
        let initial = c.translation_cache_status().unwrap();
        assert_eq!(initial.size_bytes, 8 * 1024 * 1024);
        assert_eq!(initial.allocations, 1);
        for n in 0..instructions {
            let e = c.execute(&mut b, 1).unwrap();
            assert_eq!(e.retired, 1, "instruction {n}");
            assert!(!e.waiting, "instruction {n}");
            if n % 1024 == 0 {
                b.accesses.clear();
            }
        }
        assert_eq!(c.register(0).unwrap(), instructions);
        assert_eq!(c.register(15).unwrap(), 0x100 + 4 * instructions);
        let final_cache = c.translation_cache_status().unwrap();
        assert_eq!(final_cache.size_bytes, 16 * 1024 * 1024);
        assert!(
            final_cache.allocations >= 3,
            "cache must grow and reuse its cap: {final_cache:?}"
        );
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "translation_cache_pressure_preserves_jit_prologue_and_retirement",
        ])
        .env("LABWIRED_NATIVE_CACHE_PROBE", "1")
        .spawn()
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "native cache probe exited {status}");
            break;
        }
        if std::time::Instant::now() >= until {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native execution hung under translation cache pressure");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn cache_allocation_failure_probe(mode: &str, test_name: &str) {
    if std::env::var_os("LABWIRED_NATIVE_CACHE_FAILURE").is_some() {
        let this = libloading::os::unix::Library::this();
        let failures = unsafe {
            this.get::<unsafe extern "C" fn() -> u32>(b"lw_cache_failure_count\0")
                .unwrap()
        };
        let mut b = Bus::new();
        let instructions = 200_000u32;
        b.ram.resize(0x100000, 0);
        b.words(0x100, &vec![0xe2800001; instructions as usize]);
        let mut c = ArmCpu::new().unwrap();
        c.executable_range(0, b.ram.len() as u64).unwrap();
        c.set_register(15, 0x100).unwrap();
        for n in 0..instructions {
            match c.execute(&mut b, 1) {
                Ok(e) => {
                    assert_eq!(e.retired, 1, "instruction {n}");
                    assert!(!e.waiting);
                }
                Err(error) => {
                    assert_eq!(mode, "double", "{error}");
                    assert!(
                        error.contains("Failed to reallocate code_gen_buffer"),
                        "{error}"
                    );
                    assert_eq!(unsafe { failures() }, 2);
                    assert!(c.register(0).is_err(), "failed growth must poison the CPU");
                    eprintln!("Both growth mappings failed; execution returned an error and poisoned the CPU; dropping it now");
                    drop(c);
                    return;
                }
            }
            if n % 1024 == 0 {
                b.accesses.clear();
            }
        }
        assert_eq!(
            mode, "fallback",
            "both failed mappings must return an error"
        );
        assert_eq!(unsafe { failures() }, 1);
        assert_eq!(c.register(0).unwrap(), instructions);
        assert_eq!(c.register(15).unwrap(), 0x100 + instructions * 4);
        assert!(c.translation_cache_status().unwrap().allocations >= 3);
        drop(c);
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("cache_failure_probe.c");
    let library = directory.path().join("cache_failure_probe.so");
    std::fs::write(&source, include_bytes!("cache_failure_probe.c")).unwrap();
    assert!(std::process::Command::new("cc")
        .args(["-shared", "-fPIC", "-O2", "-o"])
        .arg(&library)
        .arg(&source)
        .status()
        .unwrap()
        .success());
    let mut child = std::process::Command::new("sh")
        .args(["-c", "ulimit -c 0; exec \"$@\"", "--"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", test_name])
        .env("LABWIRED_NATIVE_CACHE_FAILURE", mode)
        .env("LD_PRELOAD", library)
        .spawn()
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "native {mode} cache probe exited {status}"
            );
            break;
        }
        if std::time::Instant::now() >= until {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native execution hung during {mode} cache allocation failure");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn failed_cache_growth_and_fallback_poison_then_drop_safely() {
    cache_allocation_failure_probe(
        "double",
        "failed_cache_growth_and_fallback_poison_then_drop_safely",
    );
}

#[test]
fn failed_cache_growth_can_retire_after_old_size_fallback() {
    cache_allocation_failure_probe(
        "fallback",
        "failed_cache_growth_can_retire_after_old_size_fallback",
    );
}

// Real guest instructions with CP10/11 access granted and ASEDIS/D32DIS clear.
// These profile fixtures do not establish general CPACR enforcement/conformance.
fn vfp_cpu(b: &mut Bus, body: &[u32], enabled: bool) -> ArmCpu {
    b.words(4, &[0xe320f003]); // WFI at the UND vector, never a success sentinel.
    let mut code = vec![0xee010f50, 0xeee80a10]; // MCR CPACR; VMSR FPEXC.
    code.extend_from_slice(body);
    code.push(0xe3a0c05a); // MOV r12, #90: reached only after the tested instructions.
    let mut c = cpu(b, &code);
    prepare_vfp(&mut c, b, enabled);
    c
}
fn prepare_vfp(c: &mut ArmCpu, b: &mut Bus, enabled: bool) {
    c.set_register(16, 0x13).unwrap(); // SVC, ARM, reset MMU remains off.
    c.set_register(15, 0x100).unwrap();
    c.set_register(0, 0x00f00000).unwrap(); // Full CP10/11, ASEDIS/D32DIS=0.
    c.execute(b, 1).unwrap();
    c.set_register(0, if enabled { 0x40000000 } else { 0 })
        .unwrap();
    c.execute(b, 1).unwrap();
    c.set_register(12, 0).unwrap();
    assert_eq!(c.register(15).unwrap(), 0x108);
    assert_eq!(c.register(16).unwrap() & 31, 0x13);
}
fn finish_vfp(c: &mut ArmCpu, b: &mut Bus, instructions: u32) {
    c.execute(b, instructions + 1).unwrap();
    // A faulting instruction can report retirement, so check architectural effects.
    assert_eq!(
        c.register(16).unwrap() & 31,
        0x13,
        "unexpected exception mode"
    );
    assert_eq!(c.register(15).unwrap(), 0x108 + (instructions + 1) * 4);
    assert_eq!(
        c.register(12).unwrap(),
        90,
        "success sentinel was not reached"
    );
}
fn ram_words(b: &Bus, address: usize, count: usize) -> Vec<u32> {
    b.ram[address..address + count * 4]
        .chunks_exact(4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
        .collect()
}
#[test]
fn cortex_a7_privileged_vfp_ids_with_fpexc_disabled_enabled_and_reset() {
    // FPSID, MVFR0, MVFR1, MIDR; retain the pinned model's r0p0 revision.
    let body = [0xeef04a10, 0xeef71a10, 0xeef62a10, 0xee103f10, 0xeef60a10];
    for enabled in [false, true] {
        let mut b = Bus::new();
        let mut c = vfp_cpu(&mut b, &body, enabled);
        for reset in [false, true] {
            if reset {
                c.reset().unwrap();
                prepare_vfp(&mut c, &mut b, enabled);
            }
            finish_vfp(&mut c, &mut b, body.len() as u32);
            for (r, expected) in [
                (4, 0x41023070),
                (0, 0x11111111),
                (1, 0x10110222),
                (2, 0x11111111),
                (3, 0x410fc070),
            ] {
                assert_eq!(
                    c.register(r).unwrap(),
                    expected,
                    "register {r}, EN={enabled}, reset={reset}"
                );
            }
        }
    }
}
#[test]
fn cortex_a7_user_vfp_ids_trap_with_fpexc_disabled_and_enabled() {
    for enabled in [false, true] {
        for opcode in [0xeef00a10, 0xeef70a10, 0xeef60a10] {
            let mut b = Bus::new();
            let mut c = vfp_cpu(&mut b, &[opcode], enabled);
            c.set_register(0, 0xdeadbeef).unwrap();
            c.set_register(16, 0x10).unwrap(); // User mode with CP10/11 still granted.
            c.execute(&mut b, 1).unwrap();
            assert_eq!(
                c.register(16).unwrap() & 31,
                0x1b,
                "opcode {opcode:x}, EN={enabled}"
            );
            assert_eq!(c.register(15).unwrap(), 4);
            assert_eq!(c.register(0).unwrap(), 0xdeadbeef);
            assert_eq!(c.register(12).unwrap(), 0);
            assert!(c.execute(&mut b, 1).unwrap().waiting);
            assert_eq!(c.register(12).unwrap(), 0);
        }
    }
}
#[test]
fn cortex_a7_vfp3_immediate_executes() {
    let mut b = Bus::new();
    // VMOV.F32 s0, #1.0; VMOV r1, s0.
    let mut c = vfp_cpu(&mut b, &[0xeeb70a00, 0xee101a10], true);
    finish_vfp(&mut c, &mut b, 2);
    assert_eq!(c.register(1).unwrap(), 0x3f800000);
}
#[test]
fn cortex_a7_d16_load_store_roundtrips_64_bits() {
    let mut b = Bus::new();
    b.words(0x300, &[0x01234567, 0x89abcdef]);
    let mut c = vfp_cpu(&mut b, &[0xedd10b00, 0xedc30b00], true);
    c.set_register(1, 0x300).unwrap();
    c.set_register(3, 0x400).unwrap();
    finish_vfp(&mut c, &mut b, 2);
    assert_eq!(ram_words(&b, 0x400, 2), [0x01234567, 0x89abcdef]);
}
#[test]
fn cortex_a7_q8_copy_roundtrips_128_bits() {
    let mut b = Bus::new();
    let input = [0x01234567, 0x89abcdef, 0x76543210, 0xfedcba98];
    b.words(0x300, &input);
    // VLDR d0/d1; VMOV q8, q0; VSTR d16/d17.
    let mut c = vfp_cpu(
        &mut b,
        &[0xed910b00, 0xed911b02, 0xf2600150, 0xedc30b00, 0xedc31b02],
        true,
    );
    c.set_register(1, 0x300).unwrap();
    c.set_register(3, 0x400).unwrap();
    finish_vfp(&mut c, &mut b, 5);
    assert_eq!(ram_words(&b, 0x400, 4), input);
}
fn neon_binary(opcode: u32, input: &[u32], expected: &[u32]) {
    let mut b = Bus::new();
    b.words(0x300, input);
    // VLDR d0..d5 (q0/q1 inputs and q2 accumulator); operation; VSTR q2.
    let mut c = vfp_cpu(
        &mut b,
        &[
            0xed910b00, 0xed911b02, 0xed912b04, 0xed913b06, 0xed914b08, 0xed915b0a, opcode,
            0xed834b00, 0xed835b02,
        ],
        true,
    );
    c.set_register(1, 0x300).unwrap();
    c.set_register(3, 0x400).unwrap();
    finish_vfp(&mut c, &mut b, 9);
    assert_eq!(ram_words(&b, 0x400, 4), expected);
}
#[test]
fn cortex_a7_neon_integer_add_executes_all_lanes() {
    neon_binary(0xf2204842, &[1, 2, 3, 4, 10, 20, 30, 40], &[11, 22, 33, 44]);
}
#[test]
fn cortex_a7_neon_float_add_executes_all_lanes() {
    let input = [1.0f32, -2.0, 0.5, 4.0, 10.0, 20.0, -0.25, -8.0].map(f32::to_bits);
    let expected = [11.0f32, 18.0, 0.25, -4.0].map(f32::to_bits);
    neon_binary(0xf2004d42, &input, &expected);
}
#[test]
fn cortex_a7_neon_fma_uses_fused_rounding() {
    // (1 + 2^-23) * (1 - 2^-23) - 1 = -2^-46; separate MUL/ADD rounds to 0.
    let mut input = [0; 12];
    input[..4].fill(0x3f800001);
    input[4..8].fill(0x3f7ffffe);
    input[8..].fill(0xbf800000);
    neon_binary(0xf2004c52, &input, &[0xa8800000; 4]);
}
#[test]
fn cortex_a7_scalar_half_conversions_preserve_unused_half() {
    for (to_half, to_float, input, intermediate) in [
        (0xeeb30a40, 0xeeb20a40, 0x3fc00000, 0x3fc03e00), // VCVTB: preserve upper half.
        (0xeeb30ac0, 0xeeb20ac0, 0x3fc00123, 0x3e000123), // VCVTT: preserve lower half.
    ] {
        let mut b = Bus::new();
        let mut c = vfp_cpu(
            &mut b,
            &[0xee000a10, to_half, 0xee101a10, to_float, 0xee102a10],
            true,
        );
        c.set_register(0, input).unwrap(); // Both round to 1.5 (0x3e00 half).
        finish_vfp(&mut c, &mut b, 5);
        assert_eq!(c.register(1).unwrap(), intermediate);
        assert_eq!(c.register(2).unwrap(), 0x3fc00000);
    }
}
#[test]
fn cortex_a7_neon_half_conversion_roundtrips_all_lanes() {
    let mut b = Bus::new();
    let input = [0x3fc00000, 0xc0000000, 0x3e800000, 0x00000000]; // 1.5, -2, .25, 0.
    b.words(0x300, &input);
    // Q0 -> packed D2 half -> Q2 float; store both representations.
    let mut c = vfp_cpu(
        &mut b,
        &[
            0xed910b00, 0xed911b02, 0xf3b62600, 0xf3b64702, 0xed832b00, 0xed834b02, 0xed835b04,
        ],
        true,
    );
    c.set_register(1, 0x300).unwrap();
    c.set_register(3, 0x400).unwrap();
    finish_vfp(&mut c, &mut b, 7);
    assert_eq!(ram_words(&b, 0x400, 2), [0xc0003e00, 0x00003400]);
    assert_eq!(ram_words(&b, 0x408, 4), input);
}
