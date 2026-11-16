// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Shared-memory transport for external-process co-simulation models.
//!
//! An opt-in alternative to JSONL pipes (`config.transport: shm`). The host
//! creates a small file in `/dev/shm`, maps it, and passes its path to the
//! child in `LABWIRED_COSIM_SHM`. A child that speaks the transport maps the
//! same file and announces itself with the one stdout line
//! [`HELLO_LINE`]; a child that does not simply stays silent and the host
//! keeps using JSONL. After the hello, every step is a futex-synchronised
//! hand-off through the mapping: no pipes, no parsing, no allocation.
//!
//! Layout (native endian, offsets in bytes; the C and Python clients under
//! `tools/cosim/` mirror it):
//!
//! ```text
//!   0  u32 magic  "LWSM"            4  u32 version (1)
//!   8  u32 n_in                    12  u32 n_out
//!  16  u32 names_off               20  u32 names_len   (NUL-terminated names:
//!  24  u32 slots_off                                    inputs, then outputs)
//!  64  u32 req_seq   (host bumps to start a step)
//!  68  u32 req_sleep (child sets while parked in futex)
//! 128  u32 resp_seq  (child copies req_seq when its outputs are ready)
//! 132  u32 resp_sleep(host sets while parked in futex)
//! 136  u32 closing   (host sets before its last req_seq bump)
//! 140  u32 fault     (child sets non-zero to report a model failure)
//! 192  u64 time_ns                200  u64 dt_ns
//! slots_off: n_in input slots then n_out output slots, 16 bytes each:
//!            u32 present, u32 kind (0 bool, 1 i64, 2 f64), u64 bits
//! ```

use memmap2::MmapMut;
use std::fs::OpenOptions;

/// The one stdout line a shared-memory-capable child prints after mapping.
pub const HELLO_LINE: &str = "{\"labwired_shm\":1}";
/// Environment variable carrying the mapping's path to the child.
pub const ENV_PATH: &str = "LABWIRED_COSIM_SHM";
pub const MAGIC: u32 = 0x4D53_574C;
pub const VERSION: u32 = 1;
pub const MAX_SIGNALS: usize = 256;

pub const OFF_N_IN: usize = 8;
pub const OFF_N_OUT: usize = 12;
pub const OFF_NAMES_OFF: usize = 16;
pub const OFF_NAMES_LEN: usize = 20;
pub const OFF_SLOTS_OFF: usize = 24;
pub const OFF_REQ_SEQ: usize = 64;
pub const OFF_REQ_SLEEP: usize = 68;
pub const OFF_RESP_SEQ: usize = 128;
pub const OFF_RESP_SLEEP: usize = 132;
pub const OFF_CLOSING: usize = 136;
pub const OFF_FAULT: usize = 140;
pub const OFF_TIME_NS: usize = 192;
pub const OFF_DT_NS: usize = 200;
pub const HEADER_BYTES: usize = 256;
pub const SLOT_BYTES: usize = 16;

pub const KIND_BOOL: u32 = 0;
pub const KIND_I64: u32 = 1;
pub const KIND_F64: u32 = 2;

/// Shared memory transport for co-simulation.
pub struct ShmTransport {
    pub mmap: MmapMut,
}

impl ShmTransport {
    pub fn new(path: &str, size: usize) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(size as u64)?;
        let mmap = unsafe { MmapMut::map_mut(&file)? };
        Ok(Self { mmap })
    }
}

#[cfg(unix)]
pub use host::ShmRegion;

#[cfg(unix)]
mod host {
    use super::*;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::{fence, AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// How long each side busy-polls before parking in the kernel. A step is a
    /// few microseconds when both sides are running, so a parked peer costs a
    /// scheduler wake-up (tens of microseconds) that spinning avoids.
    const SPIN_FOR: Duration = Duration::from_micros(150);

    /// The host's view of one mapped transport file.
    pub struct ShmRegion {
        map: MmapMut,
        path: PathBuf,
        n_in: usize,
        n_out: usize,
        slots_off: usize,
        in_names: Vec<String>,
        out_names: Vec<String>,
        seq: u32,
    }

    impl ShmRegion {
        /// Create and map a transport file for the named inputs and outputs.
        pub fn create(in_names: &[String], out_names: &[String]) -> std::io::Result<Self> {
            if in_names.len() + out_names.len() > MAX_SIGNALS {
                return Err(std::io::Error::other("too many co-sim shm signals"));
            }
            let mut names = Vec::new();
            for name in in_names.iter().chain(out_names) {
                if name.as_bytes().contains(&0) {
                    return Err(std::io::Error::other("NUL in co-sim signal name"));
                }
                names.extend_from_slice(name.as_bytes());
                names.push(0);
            }
            let n_in = in_names.len();
            let n_out = out_names.len();
            let slots_off = HEADER_BYTES;
            let names_off = slots_off + (n_in + n_out) * SLOT_BYTES;
            let total = (names_off + names.len()).next_multiple_of(4096);

            let dir = if Path::new("/dev/shm").is_dir() {
                PathBuf::from("/dev/shm")
            } else {
                std::env::temp_dir()
            };
            let path = dir.join(format!(
                "labwired-cosim-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            file.set_len(total as u64)?;
            let mut map = unsafe { MmapMut::map_mut(&file)? };
            let put = |map: &mut MmapMut, off: usize, value: u32| {
                map[off..off + 4].copy_from_slice(&value.to_ne_bytes());
            };
            put(&mut map, 0, MAGIC);
            put(&mut map, 4, VERSION);
            put(&mut map, OFF_N_IN, n_in as u32);
            put(&mut map, OFF_N_OUT, n_out as u32);
            put(&mut map, OFF_NAMES_OFF, names_off as u32);
            put(&mut map, OFF_NAMES_LEN, names.len() as u32);
            put(&mut map, OFF_SLOTS_OFF, slots_off as u32);
            map[names_off..names_off + names.len()].copy_from_slice(&names);
            Ok(Self {
                map,
                path,
                n_in,
                n_out,
                slots_off,
                in_names: in_names.to_vec(),
                out_names: out_names.to_vec(),
                seq: 0,
            })
        }

        pub fn path(&self) -> &Path {
            &self.path
        }

        /// Remove the file name. Both sides keep their mappings.
        pub fn unlink(&self) {
            let _ = std::fs::remove_file(&self.path);
        }

        pub fn in_names(&self) -> &[String] {
            &self.in_names
        }

        pub fn out_names(&self) -> &[String] {
            &self.out_names
        }

        fn word(&self, off: usize) -> &AtomicU32 {
            debug_assert!(off % 4 == 0 && off + 4 <= self.map.len());
            // SAFETY: in bounds, 4-aligned (the mapping is page aligned), and
            // only ever accessed atomically by both processes.
            unsafe { &*(self.map.as_ptr().add(off) as *const AtomicU32) }
        }

        fn read_u64(&self, off: usize) -> u64 {
            u64::from_ne_bytes(self.map[off..off + 8].try_into().unwrap())
        }

        fn write_slot(&mut self, index: usize, present: bool, kind: u32, bits: u64) {
            let off = self.slots_off + index * SLOT_BYTES;
            self.map[off + 4..off + 8].copy_from_slice(&kind.to_ne_bytes());
            self.map[off + 8..off + 16].copy_from_slice(&bits.to_ne_bytes());
            // `present` is written last so a reader never pairs a fresh flag
            // with stale bits; the seq release below publishes the lot.
            self.map[off..off + 4].copy_from_slice(&(present as u32).to_ne_bytes());
        }

        /// Set input slot `index`; `None` marks it absent.
        pub fn set_input(&mut self, index: usize, value: Option<(u32, u64)>) {
            debug_assert!(index < self.n_in);
            match value {
                Some((kind, bits)) => self.write_slot(index, true, kind, bits),
                None => self.write_slot(index, false, 0, 0),
            }
        }

        /// Output slot `index` as `(kind, bits)`, or `None` when absent.
        pub fn output(&self, index: usize) -> Option<(u32, u64)> {
            debug_assert!(index < self.n_out);
            let off = self.slots_off + (self.n_in + index) * SLOT_BYTES;
            let present = u32::from_ne_bytes(self.map[off..off + 4].try_into().unwrap());
            (present != 0).then(|| {
                (
                    u32::from_ne_bytes(self.map[off + 4..off + 8].try_into().unwrap()),
                    self.read_u64(off + 8),
                )
            })
        }

        pub fn fault(&self) -> u32 {
            self.word(OFF_FAULT).load(Ordering::Acquire)
        }

        /// Publish one request and wait for the child's matching response.
        /// `alive` is polled while parked so a dead child fails the step
        /// promptly instead of at the deadline.
        pub fn exchange(
            &mut self,
            time_ns: u64,
            dt_ns: u64,
            deadline: Instant,
            mut alive: impl FnMut() -> bool,
        ) -> Result<(), &'static str> {
            self.map[OFF_TIME_NS..OFF_TIME_NS + 8].copy_from_slice(&time_ns.to_ne_bytes());
            self.map[OFF_DT_NS..OFF_DT_NS + 8].copy_from_slice(&dt_ns.to_ne_bytes());
            self.seq = self.seq.wrapping_add(1);
            let seq = self.seq;
            self.word(OFF_REQ_SEQ).store(seq, Ordering::Release);
            fence(Ordering::SeqCst);
            if self.word(OFF_REQ_SLEEP).load(Ordering::Relaxed) != 0 {
                futex_wake(self.word(OFF_REQ_SEQ));
            }

            let resp = self.word(OFF_RESP_SEQ);
            let start = Instant::now();
            let mut spins = 0u32;
            loop {
                if resp.load(Ordering::Acquire) == seq {
                    return Ok(());
                }
                if start.elapsed() < SPIN_FOR {
                    spins = spins.wrapping_add(1);
                    // Re-check the clock only occasionally; `pause` is cheap.
                    for _ in 0..32 {
                        std::hint::spin_loop();
                    }
                    let _ = spins;
                    continue;
                }
                let now = Instant::now();
                if now >= deadline {
                    return Err("co-sim exchange deadline exceeded");
                }
                if !alive() {
                    return Err("co-sim process exited");
                }
                let sleep = self.word(OFF_RESP_SLEEP);
                sleep.store(1, Ordering::Relaxed);
                fence(Ordering::SeqCst);
                if resp.load(Ordering::Acquire) != seq {
                    let slice = (deadline - now).min(Duration::from_millis(20));
                    futex_wait(resp, resp.load(Ordering::Relaxed), slice);
                }
                sleep.store(0, Ordering::Relaxed);
            }
        }

        /// Tell a parked child to stop.
        pub fn close(&mut self) {
            self.word(OFF_CLOSING).store(1, Ordering::Release);
            self.seq = self.seq.wrapping_add(1);
            self.word(OFF_REQ_SEQ).store(self.seq, Ordering::Release);
            fence(Ordering::SeqCst);
            futex_wake(self.word(OFF_REQ_SEQ));
        }
    }

    impl Drop for ShmRegion {
        fn drop(&mut self) {
            self.unlink();
        }
    }

    #[cfg(target_os = "linux")]
    fn futex_wake(word: &AtomicU32) {
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                word as *const AtomicU32,
                libc::FUTEX_WAKE,
                i32::MAX,
                0usize,
                0usize,
                0u32,
            );
        }
    }

    /// Park until `word != expected`, a wake, or `timeout`.
    #[cfg(target_os = "linux")]
    fn futex_wait(word: &AtomicU32, expected: u32, timeout: Duration) {
        let ts = libc::timespec {
            tv_sec: timeout.as_secs() as libc::time_t,
            tv_nsec: timeout.subsec_nanos() as _,
        };
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                word as *const AtomicU32,
                libc::FUTEX_WAIT,
                expected,
                &ts as *const libc::timespec,
                0usize,
                0u32,
            );
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn futex_wake(_word: &AtomicU32) {}

    #[cfg(not(target_os = "linux"))]
    fn futex_wait(_word: &AtomicU32, _expected: u32, timeout: Duration) {
        std::thread::sleep(timeout.min(Duration::from_micros(50)));
    }
}
