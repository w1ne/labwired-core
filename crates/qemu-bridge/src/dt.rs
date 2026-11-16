// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Device-tree board loader: CPU addresses through `ranges`, interrupts
//! through `interrupt-parent` / `interrupts-extended`, `status`, and the
//! classification of every node against the model registry.

use crate::fdt::Fdt;

/// LabWired models the bridge can host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// BCM2835-family Broadcom Serial Controller (I²C master).
    Bcm2835Bsc,
}

impl ModelKind {
    pub fn name(self) -> &'static str {
        match self {
            ModelKind::Bcm2835Bsc => "bcm2835-bsc",
        }
    }
}

/// `compatible` → model. First match in the node's own compatible order wins.
/// The BCM2711 DTB lists `"brcm,bcm2711-i2c", "brcm,bcm2835-i2c"` on every
/// BSC node (i2c0/1/3/4/5/6); the BSC block is the same IP on both.
pub const REGISTRY: &[(&str, ModelKind)] = &[
    ("brcm,bcm2711-i2c", ModelKind::Bcm2835Bsc),
    ("brcm,bcm2835-i2c", ModelKind::Bcm2835Bsc),
];

/// Compatibles QEMU supplies the same way on every Arm SoC: interrupt
/// controller, architected timer, PSCI, PMU. CPUs and memory are matched by
/// `device_type` / path instead.
pub const QEMU_CORE_COMPATIBLES: &[&str] = &[
    "arm,gic-400",
    "arm,cortex-a15-gic",
    "arm,cortex-a9-gic",
    "arm,cortex-a7-gic",
    "arm,gic-v3",
    "arm,armv7-timer",
    "arm,armv8-timer",
    "arm,psci",
    "arm,psci-0.2",
    "arm,psci-1.0",
    "arm,armv8-pmuv3",
    "arm,cortex-a72-pmu",
    "arm,cortex-a53-pmu",
    "arm,cortex-a15-pmu",
    "arm,cortex-a7-pmu",
];

const GIC_COMPATIBLES: &[&str] = &[
    "arm,gic-400",
    "arm,cortex-a15-gic",
    "arm,cortex-a9-gic",
    "arm,cortex-a7-gic",
    "arm,gic-v3",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Irq {
    GicSpi(u32),
    GicPpi(u32),
    /// Not a GIC parent: controller path and raw specifier cells.
    Other {
        controller: String,
        cells: Vec<u32>,
    },
    /// interrupt-parent missing or a specifier did not parse.
    Unresolved(String),
}

impl std::fmt::Display for Irq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Irq::GicSpi(n) => write!(f, "SPI {n}"),
            Irq::GicPpi(n) => write!(f, "PPI {n}"),
            Irq::Other { controller, cells } => {
                let c: Vec<String> = cells.iter().map(|c| format!("{c:#x}")).collect();
                write!(
                    f,
                    "{}<{}>",
                    controller.rsplit('/').next().unwrap_or(""),
                    c.join(" ")
                )
            }
            Irq::Unresolved(why) => write!(f, "?({why})"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Modeled(ModelKind),
    QemuCore,
    Gap,
    Disabled,
    /// `reg` is not a CPU address (e.g. an I²C target's bus address).
    NotMmio,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::Modeled(_) => "modeled",
            Class::QemuCore => "qemu-core",
            Class::Gap => "gap",
            Class::Disabled => "disabled",
            Class::NotMmio => "not-mmio",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DtNode {
    pub path: String,
    /// Alias (e.g. `i2c1`) if /aliases names this node, else the node name.
    pub label: String,
    pub compatible: Vec<String>,
    /// First `reg` entry translated to a CPU physical address.
    pub cpu_addr: Option<u64>,
    pub size: Option<u64>,
    pub reg_entries: usize,
    pub irqs: Vec<Irq>,
    pub enabled: bool,
    pub class: Class,
}

impl DtNode {
    pub fn spi(&self) -> Option<u32> {
        self.irqs.iter().find_map(|i| match i {
            Irq::GicSpi(n) => Some(*n),
            _ => None,
        })
    }
}

pub struct Board {
    pub nodes: Vec<DtNode>,
}

fn cells_value(cells: &[u32]) -> u128 {
    cells.iter().fold(0u128, |acc, &c| (acc << 32) | c as u128)
}

impl Board {
    pub fn load(dtb: &[u8]) -> Result<Self, String> {
        let fdt = Fdt::parse(dtb)?;
        let mut nodes = Vec::new();
        for n in fdt.walk() {
            if let Some(dn) = describe(&fdt, n) {
                nodes.push(dn);
            }
        }
        Ok(Board { nodes })
    }

    pub fn by_label(&self, label: &str) -> Option<&DtNode> {
        self.nodes.iter().find(|n| n.label == label)
    }

    pub fn modeled(&self) -> impl Iterator<Item = &DtNode> {
        self.nodes
            .iter()
            .filter(|n| matches!(n.class, Class::Modeled(_)))
    }

    /// (modeled, denominator) where the denominator is every enabled node
    /// with a CPU address that QEMU core does not provide.
    pub fn coverage(&self) -> (usize, usize) {
        let modeled = self.modeled().count();
        let total = self
            .nodes
            .iter()
            .filter(|n| matches!(n.class, Class::Modeled(_) | Class::Gap))
            .count();
        (modeled, total)
    }
}

fn address_cells(fdt: &Fdt, bus: usize) -> usize {
    fdt.prop_u32(bus, "#address-cells").unwrap_or(2) as usize
}

fn size_cells(fdt: &Fdt, bus: usize) -> usize {
    fdt.prop_u32(bus, "#size-cells").unwrap_or(1) as usize
}

/// `reg` entries of `node` in its parent bus's address space.
pub fn reg_entries(fdt: &Fdt, node: usize) -> Vec<(u128, u128)> {
    let Some(parent) = fdt.nodes[node].parent else {
        return Vec::new();
    };
    let (ac, sc) = (address_cells(fdt, parent), size_cells(fdt, parent));
    let Some(cells) = fdt.prop_cells(node, "reg") else {
        return Vec::new();
    };
    if ac + sc == 0 {
        return Vec::new();
    }
    cells
        .chunks_exact(ac + sc)
        .map(|e| (cells_value(&e[..ac]), cells_value(&e[ac..])))
        .collect()
}

/// Translate an address in `node`'s parent bus space to a CPU physical
/// address, through every ancestor's `ranges`. `None` if a bus on the way
/// has no `ranges` (its children are not memory-mapped) or no entry covers
/// the address.
pub fn translate(fdt: &Fdt, node: usize, addr: u128) -> Option<u128> {
    let mut a = addr;
    let mut bus = fdt.nodes[node].parent?;
    while let Some(up) = fdt.nodes[bus].parent {
        let ranges = fdt.prop_cells(bus, "ranges")?;
        if !ranges.is_empty() {
            let (ca, pa, sz) = (
                address_cells(fdt, bus),
                address_cells(fdt, up),
                size_cells(fdt, bus),
            );
            let step = ca + pa + sz;
            if step == 0 {
                return None;
            }
            let hit = ranges.chunks_exact(step).find_map(|e| {
                let child = cells_value(&e[..ca]);
                let parent = cells_value(&e[ca..ca + pa]);
                let size = cells_value(&e[ca + pa..]);
                (a >= child && a - child < size).then(|| a - child + parent)
            })?;
            a = hit;
        }
        bus = up;
    }
    Some(a)
}

/// The interrupt parent: `interrupt-parent` on the node or the nearest
/// ancestor that has one.
pub fn interrupt_parent(fdt: &Fdt, node: usize) -> Option<usize> {
    let mut n = Some(node);
    while let Some(i) = n {
        if let Some(ph) = fdt.prop_u32(i, "interrupt-parent") {
            return fdt.by_phandle(ph);
        }
        n = fdt.nodes[i].parent;
    }
    None
}

fn decode_irq(fdt: &Fdt, ctrl: usize, cells: &[u32]) -> Irq {
    let compat = fdt.prop_strs(ctrl, "compatible");
    if compat.iter().any(|c| GIC_COMPATIBLES.contains(&c.as_str())) && cells.len() >= 2 {
        return match cells[0] {
            0 => Irq::GicSpi(cells[1]),
            1 => Irq::GicPpi(cells[1]),
            t => Irq::Unresolved(format!("GIC type {t}")),
        };
    }
    Irq::Other {
        controller: fdt.path(ctrl),
        cells: cells.to_vec(),
    }
}

/// Resolve `interrupts-extended`, else `interrupts` against the interrupt
/// parent's `#interrupt-cells`.
pub fn interrupts(fdt: &Fdt, node: usize) -> Vec<Irq> {
    if let Some(cells) = fdt.prop_cells(node, "interrupts-extended") {
        let mut out = Vec::new();
        let mut i = 0;
        while i < cells.len() {
            let Some(ctrl) = fdt.by_phandle(cells[i]) else {
                out.push(Irq::Unresolved(format!("phandle {:#x}", cells[i])));
                break;
            };
            let n = fdt.prop_u32(ctrl, "#interrupt-cells").unwrap_or(1) as usize;
            let Some(spec) = cells.get(i + 1..i + 1 + n) else {
                out.push(Irq::Unresolved("short interrupts-extended".into()));
                break;
            };
            out.push(decode_irq(fdt, ctrl, spec));
            i += 1 + n;
        }
        return out;
    }
    let Some(cells) = fdt.prop_cells(node, "interrupts") else {
        return Vec::new();
    };
    let Some(ctrl) = interrupt_parent(fdt, node) else {
        return vec![Irq::Unresolved("no interrupt-parent".into())];
    };
    let n = fdt.prop_u32(ctrl, "#interrupt-cells").unwrap_or(1) as usize;
    if n == 0 {
        return vec![Irq::Unresolved("#interrupt-cells = 0".into())];
    }
    cells
        .chunks(n)
        .map(|spec| decode_irq(fdt, ctrl, spec))
        .collect()
}

/// A node is enabled unless it, or an ancestor, has a `status` other than
/// "okay"/"ok".
pub fn enabled(fdt: &Fdt, node: usize) -> bool {
    let mut n = Some(node);
    while let Some(i) = n {
        if let Some(s) = fdt.prop_str(i, "status") {
            if s != "okay" && s != "ok" {
                return false;
            }
        }
        n = fdt.nodes[i].parent;
    }
    true
}

fn is_qemu_core(fdt: &Fdt, node: usize, path: &str, compat: &[String]) -> bool {
    let dt = fdt.prop_str(node, "device_type");
    matches!(dt.as_deref(), Some("cpu") | Some("memory"))
        || path == "/cpus"
        || path.starts_with("/cpus/")
        || path.starts_with("/reserved-memory/")
        || compat
            .iter()
            .any(|c| QEMU_CORE_COMPATIBLES.contains(&c.as_str()))
}

fn label_for(fdt: &Fdt, node: usize, path: &str) -> String {
    if let Some(aliases) = fdt.find("/aliases") {
        let mut names: Vec<&str> = fdt.nodes[aliases]
            .props
            .iter()
            .filter(|(_, v)| {
                let end = v.iter().position(|&c| c == 0).unwrap_or(v.len());
                &v[..end] == path.as_bytes()
            })
            .map(|(k, _)| k.as_str())
            .collect();
        // Prefer an indexed alias (i2c1 over i2c), then the shortest.
        names.sort_by_key(|n| (!n.ends_with(|c: char| c.is_ascii_digit()), n.len(), *n));
        if let Some(n) = names.first() {
            return (*n).to_string();
        }
    }
    fdt.nodes[node].name.replace('@', "-")
}

fn describe(fdt: &Fdt, node: usize) -> Option<DtNode> {
    if node == fdt.root() {
        return None;
    }
    let path = fdt.path(node);
    let compatible = fdt.prop_strs(node, "compatible");
    let regs = reg_entries(fdt, node);
    let core = is_qemu_core(fdt, node, &path, &compatible);
    if regs.is_empty() && !core {
        return None;
    }
    let first = regs.first().copied();
    let cpu_addr = first
        .and_then(|(a, _)| translate(fdt, node, a))
        .map(|a| a as u64);
    let size = first.map(|(_, s)| s as u64);
    let en = enabled(fdt, node);
    let class = if core {
        Class::QemuCore
    } else if cpu_addr.is_none() {
        Class::NotMmio
    } else if !en {
        Class::Disabled
    } else if let Some(kind) = compatible
        .iter()
        .find_map(|c| REGISTRY.iter().find(|(rc, _)| rc == c).map(|(_, k)| *k))
    {
        Class::Modeled(kind)
    } else {
        Class::Gap
    };
    Some(DtNode {
        label: label_for(fdt, node, &path),
        path,
        compatible,
        cpu_addr,
        size,
        reg_entries: regs.len(),
        irqs: interrupts(fdt, node),
        enabled: en,
        class,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tests/fixtures/nested.dts compiled with dtc 1.8.1.
    const NESTED: &[u8] = include_bytes!("../tests/fixtures/nested.dtb");

    fn board() -> Board {
        Board::load(NESTED).expect("fixture parses")
    }

    fn node<'a>(b: &'a Board, path: &str) -> &'a DtNode {
        b.nodes
            .iter()
            .find(|n| n.path == path)
            .unwrap_or_else(|| panic!("{path} missing"))
    }

    #[test]
    fn ranges_translate_through_nested_buses() {
        let b = board();
        // soc: 0x7e000000 -> 0xfe000000 (root 2 cells, soc 1 cell)
        assert_eq!(node(&b, "/soc/i2c@7e804000").cpu_addr, Some(0xfe80_4000));
        // soc/sub: 0x0 -> 0x7e300000 in soc space, then soc -> cpu
        assert_eq!(
            node(&b, "/soc/sub@7e300000/uart@100").cpu_addr,
            Some(0xfe30_0100)
        );
        // second soc range: 0x40000000 -> 0xff800000
        assert_eq!(node(&b, "/soc/intc@40041000").cpu_addr, Some(0xff84_1000));
        // empty ranges = identity, 2-cell address and size
        assert_eq!(
            node(&b, "/wide/blk@4,10000000").cpu_addr,
            Some(0x4_1000_0000)
        );
        assert_eq!(node(&b, "/wide/blk@4,10000000").size, Some(0x1_0000_0000));
    }

    #[test]
    fn reg_under_bus_without_ranges_is_not_mmio() {
        let b = board();
        let sensor = node(&b, "/soc/i2c@7e804000/sensor@76");
        assert_eq!(sensor.cpu_addr, None);
        assert_eq!(sensor.class, Class::NotMmio);
    }

    #[test]
    fn interrupts_resolve_through_parent_and_extended() {
        let b = board();
        // interrupt-parent inherited from the root, GIC 3 cells
        assert_eq!(node(&b, "/soc/i2c@7e804000").irqs, vec![Irq::GicSpi(117)]);
        // two specifiers in one property
        assert_eq!(
            node(&b, "/soc/sub@7e300000/uart@100").irqs,
            vec![Irq::GicSpi(121), Irq::GicPpi(9)]
        );
        // interrupts-extended across two controllers
        let gpio = node(&b, "/soc/gpio@7e200000");
        assert_eq!(gpio.irqs[0], Irq::GicSpi(113));
        assert_eq!(
            gpio.irqs[1],
            Irq::Other {
                controller: "/soc/l1intc@40000000".into(),
                cells: vec![5]
            }
        );
        // explicit interrupt-parent on the node overrides the inherited one
        assert_eq!(
            node(&b, "/soc/timer@7e003000").irqs,
            vec![Irq::Other {
                controller: "/soc/l1intc@40000000".into(),
                cells: vec![3]
            }]
        );
    }

    #[test]
    fn classification() {
        let b = board();
        assert_eq!(
            node(&b, "/soc/i2c@7e804000").class,
            Class::Modeled(ModelKind::Bcm2835Bsc)
        );
        assert_eq!(node(&b, "/soc/i2c@7e205600").class, Class::Disabled);
        assert_eq!(node(&b, "/soc/sub@7e300000/uart@100").class, Class::Gap);
        assert_eq!(node(&b, "/soc/intc@40041000").class, Class::QemuCore);
        assert_eq!(node(&b, "/cpus/cpu@0").class, Class::QemuCore);
        assert_eq!(node(&b, "/memory@0").class, Class::QemuCore);
        assert_eq!(node(&b, "/timer").class, Class::QemuCore);
        // disabled parent bus disables the child
        assert_eq!(node(&b, "/offbus/dev@10").class, Class::Disabled);
        // modeled 1 of (1 modeled + gaps)
        let (m, total) = b.coverage();
        assert_eq!(m, 1);
        assert_eq!(
            total,
            b.nodes.iter().filter(|n| n.class == Class::Gap).count() + 1
        );
    }

    #[test]
    fn labels_prefer_indexed_alias() {
        let b = board();
        assert_eq!(node(&b, "/soc/i2c@7e804000").label, "i2c1");
        assert_eq!(node(&b, "/soc/gpio@7e200000").label, "gpio-7e200000");
    }
}
