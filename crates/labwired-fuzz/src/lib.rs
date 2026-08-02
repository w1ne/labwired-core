// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Firmware fuzzing engine (Phase 1).
//!
//! Runs a firmware in the silicon-validated simulator with **edge coverage**
//! capture (AFL-style, read from the CPU PC each step — no core changes) and a
//! **crash oracle**, plus a minimal **coverage-guided** loop. The wedge: any
//! crash found here can be replayed on real silicon (HIL-confirm, Phase 3), so
//! reported crashes are silicon-true, not emulation false positives.

use anyhow::{Context, Result};
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::memory::ProgramImage;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use labwired_loader::load_elf;
use std::path::Path;

#[cfg(feature = "libafl")]
mod libafl_engine;
#[cfg(feature = "libafl")]
pub use libafl_engine::{fuzz_collect_libafl, fuzz_libafl};

/// Edge-coverage map size (AFL default — a power of two).
pub const MAP_SIZE: usize = 1 << 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Firmware signalled DONE.
    Clean,
    /// CPU fault (sim step error) or the firmware's FAULT marker.
    Crash,
    /// Ran past the step budget without finishing.
    Hang,
}

/// AFL-style hit-count edge bitmap.
pub struct CovMap(pub Vec<u8>);

impl Default for CovMap {
    fn default() -> Self {
        Self::new()
    }
}

impl CovMap {
    pub fn new() -> Self {
        Self(vec![0u8; MAP_SIZE])
    }
    /// Merge `other` into `self`; return true if `other` lit any **new** edge.
    pub fn merge_new(&mut self, other: &CovMap) -> bool {
        let mut novel = false;
        for (g, o) in self.0.iter_mut().zip(other.0.iter()) {
            if *o != 0 && *g == 0 {
                novel = true;
            }
            *g |= *o;
        }
        novel
    }
}

/// The RAM injection + verdict contract the target firmware follows.
#[derive(Clone, Copy)]
pub struct Contract {
    pub input_len: u32,
    pub input_data: u32,
    pub verdict: u32,
    pub done_magic: u32,
    pub fault_magic: u32,
}

/// A loaded fuzz target: chip + manifest + firmware image, parsed once.
pub struct Target {
    chip: ChipDescriptor,
    manifest: SystemManifest,
    image: ProgramImage,
    contract: Contract,
    max_steps: usize,
}

impl Target {
    /// Parse the chip yaml, system manifest, and firmware ELF (once).
    pub fn from_elf(
        chip_yaml: &Path,
        system_yaml: &Path,
        elf: &Path,
        contract: Contract,
        max_steps: usize,
    ) -> Result<Self> {
        let chip = ChipDescriptor::from_file(chip_yaml).context("chip")?;
        let mut manifest = SystemManifest::from_file(system_yaml).context("manifest")?;
        let anchored = system_yaml.parent().unwrap().join(&manifest.chip);
        manifest.chip = anchored.to_str().unwrap().to_string();
        let image = load_elf(elf).context("elf")?;
        Ok(Self {
            chip,
            manifest,
            image,
            contract,
            max_steps,
        })
    }

    /// Run `input` once, accumulating edge coverage into `cov`. Returns the
    /// verdict. A fresh machine per run keeps iterations independent (snapshot
    /// fast-reset is a perf follow-up; correctness first).
    pub fn run(&self, input: &[u8], cov: &mut CovMap) -> Verdict {
        let mut bus = SystemBus::from_config(&self.chip, &self.manifest).expect("bus");
        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        machine.load_firmware(&self.image).expect("load");

        // Inject the input into the RAM buffer.
        let c = &self.contract;
        machine
            .bus
            .write_u32(c.input_len as u64, input.len() as u32)
            .ok();
        for (i, chunk) in input.chunks(4).enumerate() {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            machine
                .bus
                .write_u32(
                    (c.input_data + (i as u32) * 4) as u64,
                    u32::from_le_bytes(w),
                )
                .ok();
        }

        // Step with edge coverage (AFL: idx = (prev ^ cur) & mask, prev = cur>>1).
        let mut prev: usize = 0;
        for _ in 0..self.max_steps {
            if machine.step().is_err() {
                return Verdict::Crash;
            }
            let cur = (machine.cpu.get_pc() as usize) >> 1;
            let idx = (prev ^ cur) & (MAP_SIZE - 1);
            cov.0[idx] = cov.0[idx].saturating_add(1);
            prev = cur >> 1;

            match machine.bus.read_u32(c.verdict as u64) {
                Ok(v) if v == c.done_magic => return Verdict::Clean,
                Ok(v) if v == c.fault_magic => return Verdict::Crash,
                _ => {}
            }
        }
        Verdict::Hang
    }
}

/// Deterministic xorshift64 — no system randomness (reproducible fuzzing).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// One havoc mutation of `base`.
fn mutate(base: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut v = base.to_vec();
    if v.is_empty() {
        v.push(0);
    }
    match rng.below(4) {
        0 => {
            // set a random byte to a random value
            let i = rng.below(v.len());
            v[i] = (rng.next() & 0xFF) as u8;
        }
        1 => {
            // insert a random byte
            let i = rng.below(v.len() + 1);
            v.insert(i, (rng.next() & 0xFF) as u8);
        }
        2 if v.len() > 1 => {
            // delete a byte
            let i = rng.below(v.len());
            v.remove(i);
        }
        _ => {
            // flip a random bit
            let i = rng.below(v.len());
            v[i] ^= 1 << (rng.below(8));
        }
    }
    v.truncate(256);
    v
}

/// Result of a fuzzing campaign.
pub struct FuzzReport {
    pub crash: Option<Vec<u8>>,
    pub iterations: usize,
    pub corpus_size: usize,
    pub edges_hit: usize,
}

/// Coverage-guided campaign that **collects up to `max_crashes` distinct crash
/// inputs** (deduped by bytes) rather than stopping at the first. Feeds the
/// HIL-confirm step that separates real silicon bugs from sim-only false
/// positives.
pub fn fuzz_collect(
    target: &Target,
    seeds: Vec<Vec<u8>>,
    max_iters: usize,
    seed: u64,
    max_crashes: usize,
) -> Vec<Vec<u8>> {
    #[cfg(feature = "libafl")]
    return libafl_engine::fuzz_collect_libafl(target, seeds, max_iters, seed, max_crashes);
    #[cfg(not(feature = "libafl"))]
    fuzz_collect_builtin(target, seeds, max_iters, seed, max_crashes)
}

/// Built-in (no-LibAFL) crash-collecting loop. See [`fuzz_collect`].
#[cfg_attr(feature = "libafl", allow(dead_code))]
fn fuzz_collect_builtin(
    target: &Target,
    seeds: Vec<Vec<u8>>,
    max_iters: usize,
    seed: u64,
    max_crashes: usize,
) -> Vec<Vec<u8>> {
    let mut corpus: Vec<Vec<u8>> = if seeds.is_empty() {
        vec![vec![0u8]]
    } else {
        seeds
    };
    let mut global = CovMap::new();
    for inp in &corpus {
        let mut cov = CovMap::new();
        target.run(inp, &mut cov);
        global.merge_new(&cov);
    }
    let mut crashes: Vec<Vec<u8>> = Vec::new();
    let mut rng = Rng::new(seed);
    for _ in 0..max_iters {
        if crashes.len() >= max_crashes {
            break;
        }
        let base = corpus[rng.below(corpus.len())].clone();
        let input = mutate(&base, &mut rng);
        let mut cov = CovMap::new();
        match target.run(&input, &mut cov) {
            Verdict::Crash => {
                if !crashes.contains(&input) {
                    crashes.push(input);
                }
            }
            _ => {
                if global.merge_new(&cov) {
                    corpus.push(input);
                }
            }
        }
    }
    crashes
}

/// Minimal coverage-guided fuzzer: mutate corpus inputs, keep ones that hit new
/// edges, stop on the first crash. Deterministic for a given `seed`.
pub fn fuzz(target: &Target, seeds: Vec<Vec<u8>>, max_iters: usize, seed: u64) -> FuzzReport {
    #[cfg(feature = "libafl")]
    return FuzzReport {
        crash: libafl_engine::fuzz_libafl(target, seeds, max_iters, seed),
        // LibAFL owns the loop; per-iteration bookkeeping isn't surfaced here.
        iterations: 0,
        corpus_size: 0,
        edges_hit: 0,
    };
    #[cfg(not(feature = "libafl"))]
    fuzz_builtin(target, seeds, max_iters, seed)
}

/// Built-in (no-LibAFL) coverage-guided loop. See [`fuzz`].
#[cfg_attr(feature = "libafl", allow(dead_code))]
fn fuzz_builtin(target: &Target, seeds: Vec<Vec<u8>>, max_iters: usize, seed: u64) -> FuzzReport {
    let mut corpus: Vec<Vec<u8>> = if seeds.is_empty() {
        vec![vec![0u8]]
    } else {
        seeds
    };
    let mut global = CovMap::new();
    // prime global coverage with the seeds
    for inp in &corpus {
        let mut cov = CovMap::new();
        target.run(inp, &mut cov);
        global.merge_new(&cov);
    }
    let mut rng = Rng::new(seed);
    for it in 0..max_iters {
        let base = corpus[rng.below(corpus.len())].clone();
        let input = mutate(&base, &mut rng);
        let mut cov = CovMap::new();
        match target.run(&input, &mut cov) {
            Verdict::Crash => {
                return FuzzReport {
                    crash: Some(input),
                    iterations: it + 1,
                    corpus_size: corpus.len(),
                    edges_hit: global.0.iter().filter(|&&b| b != 0).count(),
                };
            }
            _ => {
                if global.merge_new(&cov) {
                    corpus.push(input);
                }
            }
        }
    }
    FuzzReport {
        crash: None,
        iterations: max_iters,
        corpus_size: corpus.len(),
        edges_hit: global.0.iter().filter(|&&b| b != 0).count(),
    }
}
