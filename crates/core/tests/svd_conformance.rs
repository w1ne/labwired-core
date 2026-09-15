//! SVD conformance gate — chip configs vs the silicon's own register map.
//!
//! The repo vendors an SVD for nearly every supported part, and until now
//! nothing compared a chip config's *addresses* against them. `register_coverage`
//! is a coverage ratchet: it probes each SVD register and counts how many the bus
//! answers. That counts responses, not correctness — a register modelled at the
//! WRONG offset still responds, so a misplacement RAISES the coverage score. Two
//! shipped defects lived behind exactly that blind spot:
//!
//!   * stm32wb55's RCC ran the H5-style enable block (AHB2ENR@0x8C,
//!     APB1ENR1@0x9C) when RM0434 puts it at 0x4C/0x58. Every real STM32Cube /
//!     Zephyr / Arduino clock-enable write was dropped, leaving TIM1/TIM2/I2C1/
//!     SPI1/ADC1/RTC unreachable — while coverage counted 0x9C as "modelled".
//!   * stm32wba52 declared LPUART1 on NVIC 45, which is SPI1's vector, so the
//!     console UART and SPI1 dispatched through one slot.
//!
//! Neither is subtle; both survived for months because no gate ever asked the
//! SVD. This test asks it: for every chip config with a vendored SVD, each
//! peripheral's `base_address` and `irq:` must match what the silicon says.
//!
//! Deviations are allowed but must be named in `ALLOWED` with a reason. A silent
//! deviation is the failure mode this file exists to prevent, so the allow-list
//! is deliberately annoying to extend.

use std::collections::{HashMap, HashSet};

fn root(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// `configs/chips/<stem>.yaml` → vendored SVD. A chip with no SVD is simply not
/// covered; `every_svd_backed_chip_is_checked` below keeps that list honest so a
/// newly-vendored SVD cannot sit unused.
const PAIRS: &[(&str, &str)] = &[
    ("atsamd21g18a", "tests/fixtures/real_world/atsamd21g18a.svd"),
    ("esp32", "tests/fixtures/real_world/esp32.svd"),
    ("esp32c3", "tests/fixtures/real_world/esp32c3.svd"),
    ("esp32s3", "tests/fixtures/svd/esp32s3.svd"),
    ("esp32s3-zero", "tests/fixtures/svd/esp32s3.svd"),
    ("mkw41z4", "tests/fixtures/real_world/mkw41z4.svd"),
    ("nrf52832", "tests/fixtures/real_world/nrf52832.svd"),
    ("nrf52840", "tests/fixtures/real_world/nrf52840.svd"),
    ("nrf5340", "tests/fixtures/real_world/nrf5340.svd"),
    ("nrf54l15", "tests/fixtures/real_world/nrf54l15.svd"),
    ("nrf54lm20a", "tests/fixtures/real_world/nrf54lm20a.svd"),
    ("rp2040", "tests/fixtures/real_world/rp2040.svd"),
    ("stm32f103", "tests/fixtures/real_world/stm32f103.svd"),
    ("stm32f401", "tests/fixtures/real_world/stm32f401.svd"),
    ("stm32f401cdu6", "tests/fixtures/real_world/stm32f401.svd"),
    ("stm32f407", "tests/fixtures/real_world/stm32f407.svd"),
    ("stm32f411ceu6", "tests/fixtures/real_world/stm32f411.svd"),
    ("stm32g474re", "tests/fixtures/real_world/stm32g474.svd"),
    ("stm32h563", "tests/fixtures/real_world/stm32h563.svd"),
    ("stm32h735", "tests/fixtures/real_world/stm32h735.svd"),
    ("stm32l073", "tests/fixtures/real_world/stm32l073.svd"),
    ("stm32l476", "tests/fixtures/real_world/stm32l476.svd"),
    ("stm32wb55", "tests/fixtures/real_world/stm32wb55.svd"),
    ("stm32wba52", "tests/fixtures/real_world/stm32wba52.svd"),
];

/// Justified deviations: (chip, peripheral id, what, why).
///
/// Every entry is a claim that the SVD and the model disagree *and the model is
/// right*. Adding one to silence a failure you have not explained is how the
/// WB55 bug survived — the fixture had been written to match the model instead
/// of the datasheet.
const ALLOWED: &[(&str, &str, Deviation, &str)] = &[
    (
        "stm32f103",
        "bkp",
        Deviation::Base,
        "ST's F1 SVD starts the BKP block at DR1 (0x40006C04); the peripheral \
         window genuinely begins at 0x40006C00 and the 1KB region covers both.",
    ),
    // Nordic's SVDs place a GPIO port's base at its OUT register, while the
    // nRF-family GPIO model uses the block start with OUT at +0x504 (the
    // classic nRF52 layout it shares with every other Nordic part). The config
    // back-offsets the base by 0x504 so the two agree on where OUT lands. This
    // is load-bearing WHERE IT IS STILL USED: "correcting" such a base to the
    // SVD's number, without telling the model the window moved, would shift
    // every GPIO register by 0x500.
    //
    // nRF5340's two ports are no longer among them. On that part the back-offset
    // was not merely a bookkeeping convention: P0 and P1 are 0x300 apart, so a
    // window anchored 0x500 low sits inside its neighbour's and one port's
    // registers were entirely served by the other. They now sit at the SVD bases
    // with `reg_offset: 0x500` in the chip yaml, so no deviation is claimed.
    // nRF54L15 still uses the remap and still needs these entries.
    (
        "nrf54l15",
        "gpio0",
        Deviation::Base,
        "Nordic SVD bases a port at its OUT register; model uses block start.",
    ),
    (
        "nrf54l15",
        "gpio1",
        Deviation::Base,
        "Nordic SVD bases a port at its OUT register; model uses block start.",
    ),
    (
        "nrf54l15",
        "gpio2",
        Deviation::Base,
        "Nordic SVD bases a port at its OUT register; model uses block start.",
    ),
    // Blocks that silicon co-locates INSIDE another peripheral's page. The bus
    // maps one peripheral per address range, so these get a private window at a
    // reserved address. Their real registers are covered by the peripheral whose
    // page they share, and both are inert stubs, so nothing is lost today — but
    // promoting either to a behavioural model means solving the overlap first.
    (
        "nrf52840",
        "acl",
        Deviation::Base,
        "ACL's registers live in the NVMC page (0x4001E000), which `nvmc` \
         already maps; the stub gets a private window instead.",
    ),
    (
        "nrf5340",
        "fpu",
        Deviation::Base,
        "FPU's event registers live in the DCNF page (0x50000000), which `dcnf` \
         already maps; the stub sits at reserved 0x50002000 instead.",
    ),
    // KNOWN GAP, not a bookkeeping quirk: on nRF52840 the two GPIO ports share
    // one 4KB page — P0 at 0x50000000, P1 at 0x50000300 — so P1's registers
    // overlap P0's window. The GPIO model gives every port its own 4KB block, so
    // P1 cannot be mapped where silicon puts it and is parked at 0x50001000.
    // Firmware addressing P1 at its real 0x50000300 therefore lands in gpio0.
    // Closing this needs the GPIO model to support sub-page ports; until then
    // the deviation is recorded here rather than hidden.
    (
        "nrf52840",
        "gpio1",
        Deviation::Base,
        "P1 overlaps P0's 4KB page on silicon; the per-port GPIO model cannot \
         represent that, so P1 is parked at 0x50001000. KNOWN GAP.",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Deviation {
    Base,
    Irq,
}

/// Fold a peripheral name to its comparable form: Nordic's TrustZone aliases
/// (`_S` / `_NS`) and nRF54's `GLOBAL_` prefix are the same silicon block.
fn normalize(name: &str) -> String {
    let n = name.to_ascii_uppercase();
    let n = n.strip_prefix("GLOBAL_").unwrap_or(&n).to_string();
    for suffix in ["_S", "_NS"] {
        if let Some(stripped) = n.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    n
}

/// Config ids the SVD spells differently for the same block.
fn aliases(id: &str) -> Vec<String> {
    let p = id.to_ascii_uppercase();
    // `tim1_pwm` declares the pwm class alongside timer; the block is TIM1.
    let stem = p.split('_').next().unwrap_or(&p).to_string();
    let mut out = vec![p.clone(), stem.clone()];
    for base in [p, stem] {
        // Nordic: UART0→UARTE0, TWI1→TWIM1, SPI2→SPIM2, GPIO0→P0.
        if let Some(rest) = base.strip_prefix("UART") {
            out.push(format!("UARTE{rest}"));
        }
        if let Some(rest) = base.strip_prefix("TWI") {
            out.push(format!("TWIM{rest}"));
        }
        if let Some(rest) = base.strip_prefix("SPI") {
            out.push(format!("SPIM{rest}"));
        }
        if let Some(rest) = base.strip_prefix("GPIO") {
            out.push(format!("P{rest}"));
        }
        // ST: USART2 vs UART2 spellings vary by family.
        if let Some(rest) = base.strip_prefix("USART") {
            out.push(format!("UART{rest}"));
        }
        if let Some(rest) = base.strip_prefix("UART") {
            out.push(format!("USART{rest}"));
        }
    }
    out
}

struct SvdFacts {
    /// normalized peripheral name → every base address it appears at (a part may
    /// expose secure + non-secure aliases of one block).
    bases: HashMap<String, HashSet<u64>>,
    /// normalized peripheral name → every NVIC number it owns. A block may own
    /// several (TIM1 has BRK/UP/TRG_COM/CC), so any of them is acceptable.
    irqs: HashMap<String, HashSet<u32>>,
}

fn load_svd(rel: &str) -> SvdFacts {
    let xml = std::fs::read_to_string(root(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
    // Shared with the CLI importer and the coverage scan — see
    // `svd_ingestor::parse_svd`. A parse failure is a hard error here: silently
    // skipping an unparseable SVD is how a gate quietly stops covering a chip.
    let device = svd_ingestor::parse_svd(&xml).unwrap_or_else(|e| panic!("parse {rel}: {e}"));
    let mut bases: HashMap<String, HashSet<u64>> = HashMap::new();
    let mut irqs: HashMap<String, HashSet<u32>> = HashMap::new();
    let mut names = HashSet::new();
    for p in &device.peripherals {
        let name = normalize(&p.name);
        bases
            .entry(name.clone())
            .or_default()
            .insert(p.base_address);
        names.insert(name);
    }

    // Which block owns an interrupt is decided by the interrupt's NAME, not by
    // the peripheral it is nested under. ST's files are unreliable on the
    // nesting — the L073 SVD declares an interrupt named `TIM3` inside the
    // `TIM6` peripheral and one named `USART2` inside `USART5` — so trusting
    // the nesting invents defects that are not there.
    //
    // A shared vector names every block it serves (`USART4_USART5`,
    // `HASH_RNG`, `TIM6_DAC`), so split on `_` and credit each token that names
    // a real peripheral. Only when NO token names a peripheral — e.g. H735's
    // `TIM_CC`, which is TIM1's capture/compare vector but spells no peripheral
    // — does the nesting become the best evidence available, and only then is
    // it used.
    for p in &device.peripherals {
        let declaring = normalize(&p.name);
        for i in &p.interrupt {
            let claimants: Vec<String> = normalize(&i.name)
                .split('_')
                .map(|t| t.to_string())
                .filter(|t| names.contains(t))
                .collect();
            if claimants.is_empty() {
                irqs.entry(declaring.clone()).or_default().insert(i.value);
            } else {
                for c in claimants {
                    irqs.entry(c).or_default().insert(i.value);
                }
            }
        }
    }
    SvdFacts { bases, irqs }
}

fn allowed(chip: &str, id: &str, kind: Deviation) -> bool {
    ALLOWED
        .iter()
        .any(|(c, p, k, _)| *c == chip && *p == id && *k == kind)
}

#[test]
fn chip_configs_match_their_svd() {
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for (chip, svd) in PAIRS {
        let facts = load_svd(svd);
        let cfg_path = root(&format!("configs/chips/{chip}.yaml"));
        let chip_cfg = labwired_config::ChipDescriptor::from_file(&cfg_path)
            .unwrap_or_else(|e| panic!("load configs/chips/{chip}.yaml: {e}"));

        for p in &chip_cfg.peripherals {
            // Cortex-M core blocks (SysTick/NVIC/SCB) are architectural, not in
            // the vendor SVD's peripheral list; their addresses are fixed by ARM.
            let names = aliases(&p.id);
            let Some(name) = names.iter().find(|n| facts.bases.contains_key(*n)) else {
                continue;
            };
            checked += 1;

            let svd_bases = &facts.bases[name];
            if !svd_bases.contains(&p.base_address) && !allowed(chip, &p.id, Deviation::Base) {
                let mut want: Vec<_> = svd_bases.iter().map(|b| format!("{b:#X}")).collect();
                want.sort();
                failures.push(format!(
                    "{chip}/{}: base {:#X}, SVD[{name}] = {}",
                    p.id,
                    p.base_address,
                    want.join(" | ")
                ));
            }

            if let Some(irq) = p.irq {
                if let Some(svd_irqs) = facts.irqs.get(name) {
                    if !svd_irqs.contains(&irq) && !allowed(chip, &p.id, Deviation::Irq) {
                        let mut want: Vec<_> = svd_irqs.iter().copied().collect();
                        want.sort_unstable();
                        failures.push(format!(
                            "{chip}/{}: irq {irq}, SVD[{name}] = {want:?}",
                            p.id
                        ));
                    }
                }
            }
        }
    }

    assert!(
        checked > 150,
        "only {checked} peripherals matched an SVD block — the name matching \
         has probably broken, which would make this gate pass vacuously"
    );
    assert!(
        failures.is_empty(),
        "chip configs disagree with their SVD ({} findings; {checked} peripherals \
         checked).\n  {}\n\nThe SVD is the oracle. Fix the config, or — if the \
         model is genuinely right — add an entry to ALLOWED with a reason.",
        failures.len(),
        failures.join("\n  ")
    );
}

/// A vendored SVD that no chip is checked against is a gate that does nothing.
#[test]
fn every_vendored_svd_is_used_by_the_gate() {
    let used: HashSet<&str> = PAIRS.iter().map(|(_, s)| *s).collect();
    let mut orphans = Vec::new();
    for dir in ["tests/fixtures/real_world", "tests/fixtures/svd"] {
        for entry in std::fs::read_dir(root(dir)).expect("read svd dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_none_or(|e| e != "svd") {
                continue;
            }
            let rel = format!("{dir}/{}", path.file_name().unwrap().to_string_lossy());
            if !used.contains(rel.as_str()) {
                orphans.push(rel);
            }
        }
    }
    orphans.sort();
    assert!(
        orphans.is_empty(),
        "vendored SVDs no chip is checked against: {orphans:?}. Either wire the \
         matching chip into PAIRS or delete the file — an unused oracle is worse \
         than no oracle, because it looks like coverage."
    );
}
