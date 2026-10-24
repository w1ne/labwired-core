// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Variable values: evaluate each in-scope variable's DWARF location
//! expression against a frame's registers and the target's memory, then
//! format the bytes by the variable's DWARF type as a tree.
//!
//! A variable is unavailable only when DWARF says it is optimized out at the
//! pc, or when a register or memory it needs cannot be read.

use crate::{DwarfReader, SymbolProvider, VarScope};
use gimli::{
    AttributeValue, DebuggingInformationEntry, EvaluationResult, Location, Reader, UnitOffset,
};
use serde::Serialize;

type Unit = gimli::Unit<DwarfReader>;
type Entry<'a, 'u> = DebuggingInformationEntry<'a, 'u, DwarfReader>;

/// Most nested levels of a value tree (a variable is level 0).
const MAX_DEPTH: usize = 4;
/// Most nodes in the tree of one frame's variables.
const MAX_NODES: usize = 4000;
/// Most children shown for one array.
const MAX_ARRAY_CHILDREN: u64 = 64;
/// Most bytes a C string or `&str` preview reads.
const MAX_STRING: usize = 64;

/// Reads target memory without side effects that matter to the program.
pub trait TargetMemory {
    /// Fill `buf` from `addr`. False when any byte is unreadable.
    fn read_memory(&self, addr: u64, buf: &mut [u8]) -> bool;
}

/// Register values by DWARF register number; `None` where not known.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegisterSet(Vec<Option<u64>>);

impl RegisterSet {
    pub fn get(&self, reg: u16) -> Option<u64> {
        self.0.get(reg as usize).copied().flatten()
    }

    pub fn set(&mut self, reg: u16, value: Option<u64>) {
        let i = reg as usize;
        if self.0.len() <= i {
            self.0.resize(i + 1, None);
        }
        self.0[i] = value;
    }

    /// One past the highest register number held.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One variable (or a member or element of one) with its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VariableValue {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    /// The display value: `42`, `true`, `0x20000010`, `Game { … }`,
    /// `[u8; 504]`, or `<optimized out>` when not available.
    pub value: String,
    /// False only when DWARF says the value is optimized out here, or a
    /// register or memory it needs cannot be read.
    pub available: bool,
    /// Where the value lives, when it is in memory.
    pub address: Option<u64>,
    pub children: Vec<VariableValue>,
}

/// The machine state of one frame, for evaluating its variables.
pub struct FrameContext<'a> {
    /// The address to select location lists by: the pc, or for a caller a
    /// pc inside the call instruction.
    pub pc: u64,
    pub regs: &'a crate::RegisterSet,
    /// The frame's canonical frame address, for `DW_OP_call_frame_cfa`.
    pub cfa: Option<u64>,
    pub mem: &'a dyn TargetMemory,
    /// Width of a register in bytes.
    pub register_size: u8,
}

/// Where a value's bytes are.
#[derive(Clone)]
enum Src {
    /// In target memory at this address.
    Mem(u64),
    /// Assembled from registers, constants and pieces; `None` bytes are
    /// optimized out.
    Bytes(Vec<Option<u8>>),
}

/// Why a value has no bytes.
#[derive(Clone, Copy)]
enum Missing {
    OptimizedOut,
    Unreadable,
}

impl Missing {
    fn text(&self) -> &'static str {
        match self {
            Missing::OptimizedOut => "<optimized out>",
            Missing::Unreadable => "<not available>",
        }
    }
}

impl Src {
    fn at(&self, offset: u64) -> Src {
        match self {
            Src::Mem(a) => Src::Mem(a.wrapping_add(offset)),
            Src::Bytes(b) => Src::Bytes(b.iter().skip(offset as usize).copied().collect()),
        }
    }

    fn address(&self) -> Option<u64> {
        match self {
            Src::Mem(a) => Some(*a),
            Src::Bytes(_) => None,
        }
    }

    fn read(&self, mem: &dyn TargetMemory, len: usize) -> Result<Vec<u8>, Missing> {
        match self {
            Src::Mem(a) => {
                let mut buf = vec![0u8; len];
                if mem.read_memory(*a, &mut buf) {
                    Ok(buf)
                } else {
                    Err(Missing::Unreadable)
                }
            }
            Src::Bytes(b) => {
                if b.len() < len {
                    return Err(Missing::OptimizedOut);
                }
                b[..len]
                    .iter()
                    .map(|x| x.ok_or(Missing::OptimizedOut))
                    .collect()
            }
        }
    }
}

fn le(bytes: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    let n = bytes.len().min(8);
    buf[..n].copy_from_slice(&bytes[..n]);
    u64::from_le_bytes(buf)
}

fn sign_extend(v: u64, bytes: usize) -> i64 {
    if bytes >= 8 {
        return v as i64;
    }
    let shift = 64 - bytes * 8;
    ((v << shift) as i64) >> shift
}

fn value_bytes(v: gimli::Value) -> Vec<u8> {
    use gimli::Value;
    match v {
        Value::Generic(x) => x.to_le_bytes().to_vec(),
        Value::I8(x) => x.to_le_bytes().to_vec(),
        Value::U8(x) => x.to_le_bytes().to_vec(),
        Value::I16(x) => x.to_le_bytes().to_vec(),
        Value::U16(x) => x.to_le_bytes().to_vec(),
        Value::I32(x) => x.to_le_bytes().to_vec(),
        Value::U32(x) => x.to_le_bytes().to_vec(),
        Value::I64(x) => x.to_le_bytes().to_vec(),
        Value::U64(x) => x.to_le_bytes().to_vec(),
        Value::F32(x) => x.to_bits().to_le_bytes().to_vec(),
        Value::F64(x) => x.to_bits().to_le_bytes().to_vec(),
    }
}

/// Registers and memory an expression evaluation reads.
struct EvalEnv<'a> {
    regs: &'a crate::RegisterSet,
    mem: &'a dyn TargetMemory,
    frame_base: Option<u64>,
    cfa: Option<u64>,
}

/// Runs `eval` to completion, servicing its requests from `env`.
fn run_evaluation(
    eval: &mut gimli::Evaluation<DwarfReader>,
    env: &EvalEnv<'_>,
    dwarf: Option<(&gimli::Dwarf<DwarfReader>, &Unit)>,
) -> Result<(), Missing> {
    let mut result = eval.evaluate().map_err(|_| Missing::Unreadable)?;
    for _ in 0..1024 {
        result = match result {
            EvaluationResult::Complete => return Ok(()),
            EvaluationResult::RequiresMemory { address, size, .. } => {
                let mut buf = [0u8; 8];
                let n = (size as usize).clamp(1, 8);
                if !env.mem.read_memory(address, &mut buf[..n]) {
                    return Err(Missing::Unreadable);
                }
                eval.resume_with_memory(gimli::Value::Generic(u64::from_le_bytes(buf)))
            }
            EvaluationResult::RequiresRegister { register, .. } => {
                let v = env.regs.get(register.0).ok_or(Missing::Unreadable)?;
                eval.resume_with_register(gimli::Value::Generic(v))
            }
            EvaluationResult::RequiresFrameBase => {
                eval.resume_with_frame_base(env.frame_base.ok_or(Missing::Unreadable)?)
            }
            EvaluationResult::RequiresCallFrameCfa => {
                eval.resume_with_call_frame_cfa(env.cfa.ok_or(Missing::Unreadable)?)
            }
            EvaluationResult::RequiresRelocatedAddress(a) => eval.resume_with_relocated_address(a),
            EvaluationResult::RequiresIndexedAddress { index, .. } => {
                let (dwarf, unit) = dwarf.ok_or(Missing::Unreadable)?;
                let a = dwarf
                    .address(unit, index)
                    .map_err(|_| Missing::Unreadable)?;
                eval.resume_with_indexed_address(a)
            }
            EvaluationResult::RequiresBaseType(offset) => {
                let (_, unit) = dwarf.ok_or(Missing::Unreadable)?;
                let entry = unit.entry(offset).map_err(|_| Missing::Unreadable)?;
                let encoding = match entry.attr_value(gimli::DW_AT_encoding) {
                    Ok(Some(AttributeValue::Encoding(e))) => e,
                    _ => return Err(Missing::Unreadable),
                };
                let size = udata(&entry, gimli::DW_AT_byte_size).unwrap_or(0);
                let ty = gimli::ValueType::from_encoding(encoding, size)
                    .unwrap_or(gimli::ValueType::Generic);
                eval.resume_with_base_type(ty)
            }
            // The value on entry to the function, or a parameter's value
            // at the call site: not recoverable here.
            EvaluationResult::RequiresEntryValue(_) | EvaluationResult::RequiresParameterRef(_) => {
                return Err(Missing::OptimizedOut)
            }
            EvaluationResult::RequiresTls(_) | EvaluationResult::RequiresAtLocation(_) => {
                return Err(Missing::Unreadable)
            }
        }
        .map_err(|_| Missing::Unreadable)?;
    }
    Err(Missing::Unreadable)
}

/// Evaluates a CFI expression (a CFA rule, or a register rule with the CFA
/// pushed first) to the value it leaves.
pub(crate) fn eval_cfi(
    expr: gimli::Expression<DwarfReader>,
    cfa: Option<u64>,
    regs: &crate::RegisterSet,
    mem: &dyn TargetMemory,
    address_size: u8,
) -> Option<u64> {
    let encoding = gimli::Encoding {
        format: gimli::Format::Dwarf32,
        version: 4,
        address_size,
    };
    let mut eval = expr.evaluation(encoding);
    if let Some(cfa) = cfa {
        eval.set_initial_value(cfa);
    }
    let env = EvalEnv {
        regs,
        mem,
        frame_base: None,
        cfa,
    };
    run_evaluation(&mut eval, &env, None).ok()?;
    match eval.result().first()?.location {
        Location::Address { address } => Some(address),
        Location::Value { value } => value.to_u64(!0).ok(),
        Location::Register { register } => regs.get(register.0),
        _ => None,
    }
}

fn udata(entry: &Entry<'_, '_>, at: gimli::DwAt) -> Option<u64> {
    entry.attr_value(at).ok().flatten()?.udata_value()
}

/// The DIE in `unit` that `value` refers to, when it is in the same unit.
fn unit_ref(unit: &Unit, value: AttributeValue<DwarfReader>) -> Option<UnitOffset> {
    match value {
        AttributeValue::UnitRef(o) => Some(o),
        AttributeValue::DebugInfoRef(o) => o.to_unit_offset(&unit.header),
        _ => None,
    }
}

/// One member of a struct, union or variant.
struct Member {
    name: String,
    ty: Option<UnitOffset>,
    offset: u64,
    /// `(bit offset from the member's storage start, bit size)` of a bit field.
    bits: Option<(u64, u64)>,
}

/// Formats values of one compilation unit's types.
struct Formatter<'a> {
    provider: &'a SymbolProvider,
    unit: &'a Unit,
    mem: &'a dyn TargetMemory,
    rust: bool,
    nodes: usize,
}

impl SymbolProvider {
    /// The variables and parameters in scope at `ctx.pc`, with their values
    /// in that frame. With `level`, only those of that inline level (0 is
    /// the physical function, 1 the call inlined into it, and so on).
    pub fn variables(&self, ctx: &FrameContext<'_>, level: Option<usize>) -> Vec<VariableValue> {
        let mut out = Vec::new();
        let mut nodes = 0usize;
        self.walk_scope(ctx.pc, |unit, entry, scope| {
            if level.is_some_and(|l| l != scope.level) {
                return;
            }
            let mut fmt = Formatter {
                provider: self,
                unit,
                mem: ctx.mem,
                rust: is_rust(unit),
                nodes,
            };
            if let Some(v) = fmt.variable(entry, scope, ctx) {
                out.push(v);
            }
            nodes = fmt.nodes;
        });
        out
    }

    /// A DIE attribute, or that of the DIE it is an inlined or out-of-line
    /// instance of (`DW_AT_abstract_origin`, `DW_AT_specification`).
    fn attr_origin(
        &self,
        unit: &Unit,
        entry: &Entry<'_, '_>,
        at: gimli::DwAt,
    ) -> Option<AttributeValue<DwarfReader>> {
        if let Ok(Some(v)) = entry.attr_value(at) {
            return Some(v);
        }
        for link in [gimli::DW_AT_abstract_origin, gimli::DW_AT_specification] {
            if let Some(offset) = entry
                .attr_value(link)
                .ok()
                .flatten()
                .and_then(|v| unit_ref(unit, v))
            {
                if let Ok(origin) = unit.entry(offset) {
                    if let Ok(Some(v)) = origin.attr_value(at) {
                        return Some(v);
                    }
                }
            }
        }
        None
    }

    fn string(&self, unit: &Unit, value: AttributeValue<DwarfReader>) -> Option<String> {
        let s = self.dwarf.attr_string(unit, value).ok()?;
        s.to_string_lossy().ok().map(|c| c.into_owned())
    }

    /// `DW_AT_frame_base` of the subprogram at `offset`, evaluated in `ctx`.
    fn frame_base(&self, unit: &Unit, offset: UnitOffset, ctx: &FrameContext<'_>) -> Option<u64> {
        let entry = unit.entry(offset).ok()?;
        let AttributeValue::Exprloc(expr) = entry.attr_value(gimli::DW_AT_frame_base).ok()?? else {
            return None;
        };
        let env = EvalEnv {
            regs: ctx.regs,
            mem: ctx.mem,
            frame_base: None,
            cfa: ctx.cfa,
        };
        let mut eval = expr.evaluation(unit.encoding());
        run_evaluation(&mut eval, &env, Some((&self.dwarf, unit))).ok()?;
        match eval.result().first()?.location {
            Location::Register { register } => ctx.regs.get(register.0),
            Location::Address { address } => Some(address),
            Location::Value { value } => value.to_u64(!0).ok(),
            _ => None,
        }
    }
}

fn is_rust(unit: &Unit) -> bool {
    let mut entries = unit.entries();
    let Ok(Some((_, root))) = entries.next_dfs() else {
        return false;
    };
    matches!(
        root.attr_value(gimli::DW_AT_language),
        Ok(Some(AttributeValue::Language(gimli::DW_LANG_Rust)))
    )
}

impl<'a> Formatter<'a> {
    fn dwarf(&self) -> &'a gimli::Dwarf<DwarfReader> {
        &self.provider.dwarf
    }

    fn address_size(&self) -> u64 {
        u64::from(self.unit.encoding().address_size)
    }

    /// One in-scope variable: its location at the pc, then its value.
    fn variable(
        &mut self,
        entry: &Entry<'_, '_>,
        scope: VarScope,
        ctx: &FrameContext<'_>,
    ) -> Option<VariableValue> {
        let unit = self.unit;
        let p = self.provider;
        let name = p
            .attr_origin(unit, entry, gimli::DW_AT_name)
            .and_then(|v| p.string(unit, v))?;
        let ty = p
            .attr_origin(unit, entry, gimli::DW_AT_type)
            .and_then(|v| unit_ref(unit, v));
        let unavailable = |fmt: &Self, why: Missing| VariableValue {
            name: name.clone(),
            type_name: ty.map(|t| fmt.type_name(t, 0)),
            value: why.text().into(),
            available: false,
            address: None,
            children: Vec::new(),
        };
        // A constant the compiler folded away keeps its value here.
        if let Ok(Some(c)) = entry.attr_value(gimli::DW_AT_const_value) {
            let bytes = match c {
                AttributeValue::Block(b) => b.to_slice().ok()?.to_vec(),
                AttributeValue::Sdata(v) => v.to_le_bytes().to_vec(),
                AttributeValue::Udata(v) => v.to_le_bytes().to_vec(),
                other => other.udata_value()?.to_le_bytes().to_vec(),
            };
            let src = Src::Bytes(bytes.into_iter().map(Some).collect());
            return Some(self.value(name, ty, src, 0));
        }
        let expr = match entry.attr_value(gimli::DW_AT_location).ok()? {
            Some(AttributeValue::Exprloc(expr)) => expr,
            Some(value) => {
                let Ok(Some(mut list)) = self.dwarf().attr_locations(unit, value) else {
                    return Some(unavailable(self, Missing::OptimizedOut));
                };
                let mut found = None;
                while let Ok(Some(e)) = list.next() {
                    if e.range.begin <= ctx.pc && ctx.pc < e.range.end {
                        found = Some(e.data);
                        break;
                    }
                }
                match found {
                    Some(expr) => expr,
                    None => return Some(unavailable(self, Missing::OptimizedOut)),
                }
            }
            // Declared but with no location at all: optimized out.
            None => return Some(unavailable(self, Missing::OptimizedOut)),
        };
        let env = EvalEnv {
            regs: ctx.regs,
            mem: ctx.mem,
            frame_base: p.frame_base(unit, scope.subprogram, ctx),
            cfa: ctx.cfa,
        };
        let mut eval = expr.evaluation(unit.encoding());
        if let Err(why) = run_evaluation(&mut eval, &env, Some((self.dwarf(), unit))) {
            return Some(unavailable(self, why));
        }
        let pieces = eval.result();
        let size = ty.and_then(|t| self.type_size(t, 0)).unwrap_or(4) as usize;
        let src = match self.assemble(&pieces, size, ctx) {
            Ok(src) => src,
            Err(why) => return Some(unavailable(self, why)),
        };
        Some(self.value(name, ty, src, 0))
    }

    /// The bytes a location description's pieces describe.
    fn assemble(
        &self,
        pieces: &[gimli::Piece<DwarfReader>],
        size: usize,
        ctx: &FrameContext<'_>,
    ) -> Result<Src, Missing> {
        let reg_size = usize::from(ctx.register_size.max(1));
        // `count` bytes of register `reg` and those after it.
        let register_bytes = |reg: u16, count: usize| -> Vec<Option<u8>> {
            let mut out = Vec::with_capacity(count);
            let mut r = reg;
            while out.len() < count {
                match ctx.regs.get(r) {
                    Some(v) => out.extend(v.to_le_bytes()[..reg_size].iter().map(|b| Some(*b))),
                    None => out.extend(std::iter::repeat_n(None, reg_size)),
                }
                r += 1;
            }
            out.truncate(count);
            out
        };
        if let [piece] = pieces {
            if piece.size_in_bits.is_none() {
                return match &piece.location {
                    Location::Empty => Err(Missing::OptimizedOut),
                    Location::Address { address } => Ok(Src::Mem(*address)),
                    Location::Register { register } => {
                        let bytes = register_bytes(register.0, size.max(1));
                        if bytes.iter().all(Option::is_none) {
                            Err(Missing::Unreadable)
                        } else {
                            Ok(Src::Bytes(bytes))
                        }
                    }
                    Location::Value { value } => Ok(Src::Bytes(
                        value_bytes(*value).into_iter().map(Some).collect(),
                    )),
                    Location::Bytes { value } => Ok(Src::Bytes(
                        value
                            .to_slice()
                            .map_err(|_| Missing::Unreadable)?
                            .iter()
                            .map(|b| Some(*b))
                            .collect(),
                    )),
                    Location::ImplicitPointer { .. } => Err(Missing::OptimizedOut),
                };
            }
        }
        let mut out: Vec<Option<u8>> = Vec::with_capacity(size);
        for piece in pieces {
            let n = piece
                .size_in_bits
                .map(|b| b.div_ceil(8) as usize)
                .unwrap_or(size.saturating_sub(out.len()));
            match &piece.location {
                Location::Address { address } => {
                    let mut buf = vec![0u8; n];
                    if ctx.mem.read_memory(*address, &mut buf) {
                        out.extend(buf.into_iter().map(Some));
                    } else {
                        out.extend(std::iter::repeat_n(None, n));
                    }
                }
                Location::Register { register } => out.extend(register_bytes(register.0, n)),
                Location::Value { value } => {
                    let mut b: Vec<Option<u8>> =
                        value_bytes(*value).into_iter().map(Some).collect();
                    b.resize(n, Some(0));
                    out.extend(b);
                }
                Location::Bytes { value } => {
                    let slice = value.to_slice().map_err(|_| Missing::Unreadable)?;
                    let mut b: Vec<Option<u8>> = slice.iter().map(|x| Some(*x)).collect();
                    b.resize(n, None);
                    out.extend(b);
                }
                Location::Empty | Location::ImplicitPointer { .. } => {
                    out.extend(std::iter::repeat_n(None, n))
                }
            }
        }
        if out.iter().all(Option::is_none) {
            return Err(Missing::OptimizedOut);
        }
        Ok(Src::Bytes(out))
    }

    fn entry(&self, offset: UnitOffset) -> Option<Entry<'a, 'a>> {
        self.unit.entry(offset).ok()
    }

    fn name_of(&self, entry: &Entry<'_, '_>) -> Option<String> {
        let v = entry.attr_value(gimli::DW_AT_name).ok()??;
        self.provider.string(self.unit, v)
    }

    fn type_of(&self, entry: &Entry<'_, '_>) -> Option<UnitOffset> {
        unit_ref(self.unit, entry.attr_value(gimli::DW_AT_type).ok()??)
    }

    /// The type under typedefs and cv-qualifiers.
    fn strip(&self, mut offset: UnitOffset) -> Option<(UnitOffset, Entry<'a, 'a>)> {
        for _ in 0..16 {
            let entry = self.entry(offset)?;
            match entry.tag() {
                gimli::DW_TAG_typedef
                | gimli::DW_TAG_const_type
                | gimli::DW_TAG_volatile_type
                | gimli::DW_TAG_restrict_type
                | gimli::DW_TAG_atomic_type => match self.type_of(&entry) {
                    Some(next) => offset = next,
                    None => return Some((offset, entry)),
                },
                _ => return Some((offset, entry)),
            }
        }
        None
    }

    /// The array dimensions (element counts) of an array type.
    fn dimensions(&self, offset: UnitOffset) -> Vec<Option<u64>> {
        let mut dims = Vec::new();
        let Ok(mut tree) = self.unit.entries_tree(Some(offset)) else {
            return dims;
        };
        let Ok(root) = tree.root() else { return dims };
        let mut children = root.children();
        while let Ok(Some(child)) = children.next() {
            let e = child.entry();
            if e.tag() != gimli::DW_TAG_subrange_type {
                continue;
            }
            let count = udata(e, gimli::DW_AT_count).or_else(|| {
                let lower = udata(e, gimli::DW_AT_lower_bound).unwrap_or(0);
                let upper = match e.attr_value(gimli::DW_AT_upper_bound).ok()?? {
                    AttributeValue::Sdata(v) if v < 0 => return Some(0),
                    v => v.udata_value()?,
                };
                Some(upper + 1 - lower)
            });
            dims.push(count);
        }
        dims
    }

    fn type_size(&self, offset: UnitOffset, depth: usize) -> Option<u64> {
        if depth > 16 {
            return None;
        }
        let (offset, entry) = self.strip(offset)?;
        if let Some(size) = udata(&entry, gimli::DW_AT_byte_size) {
            return Some(size);
        }
        match entry.tag() {
            gimli::DW_TAG_pointer_type
            | gimli::DW_TAG_reference_type
            | gimli::DW_TAG_rvalue_reference_type
            | gimli::DW_TAG_ptr_to_member_type => Some(self.address_size()),
            gimli::DW_TAG_array_type => {
                let elem = self.type_size(self.type_of(&entry)?, depth + 1)?;
                let stride = udata(&entry, gimli::DW_AT_byte_stride).unwrap_or(elem);
                let mut n = 1u64;
                for d in self.dimensions(offset) {
                    n = n.saturating_mul(d?);
                }
                Some(stride.saturating_mul(n))
            }
            gimli::DW_TAG_enumeration_type => self.type_size(self.type_of(&entry)?, depth + 1),
            _ => None,
        }
    }

    /// A type's display name.
    fn type_name(&self, offset: UnitOffset, depth: usize) -> String {
        if depth > 8 {
            return "…".into();
        }
        let Some(entry) = self.entry(offset) else {
            return "?".into();
        };
        if let Some(name) = self.name_of(&entry) {
            return name;
        }
        let inner = |fmt: &Self| {
            fmt.type_of(&entry)
                .map(|t| fmt.type_name(t, depth + 1))
                .unwrap_or_else(|| "void".into())
        };
        match entry.tag() {
            gimli::DW_TAG_pointer_type if self.rust => format!("*const {}", inner(self)),
            gimli::DW_TAG_pointer_type => format!("{} *", inner(self)),
            gimli::DW_TAG_reference_type => format!("&{}", inner(self)),
            gimli::DW_TAG_rvalue_reference_type => format!("{}&&", inner(self)),
            gimli::DW_TAG_const_type => format!("const {}", inner(self)),
            gimli::DW_TAG_volatile_type => format!("volatile {}", inner(self)),
            gimli::DW_TAG_restrict_type => format!("{} restrict", inner(self)),
            gimli::DW_TAG_atomic_type => format!("_Atomic {}", inner(self)),
            gimli::DW_TAG_array_type => {
                let dims: String = self
                    .dimensions(offset)
                    .iter()
                    .map(|d| d.map(|n| n.to_string()).unwrap_or_default())
                    .map(|d| {
                        if self.rust {
                            format!("; {d}]")
                        } else {
                            format!("[{d}]")
                        }
                    })
                    .collect();
                if self.rust {
                    let n = self.dimensions(offset).len();
                    format!("{}{}{}", "[".repeat(n), inner(self), dims)
                } else {
                    format!("{}{}", inner(self), dims)
                }
            }
            gimli::DW_TAG_subroutine_type => "fn".into(),
            gimli::DW_TAG_structure_type => "struct".into(),
            gimli::DW_TAG_union_type => "union".into(),
            gimli::DW_TAG_enumeration_type => "enum".into(),
            _ => "?".into(),
        }
    }

    fn leaf(
        name: String,
        type_name: Option<String>,
        src: &Src,
        value: Result<String, Missing>,
    ) -> VariableValue {
        let (value, available) = match value {
            Ok(v) => (v, true),
            Err(why) => (why.text().to_string(), false),
        };
        VariableValue {
            name,
            type_name,
            value,
            available,
            address: src.address(),
            children: Vec::new(),
        }
    }

    /// The value of type `ty` at `src`, as a tree.
    fn value(
        &mut self,
        name: String,
        ty: Option<UnitOffset>,
        src: Src,
        depth: usize,
    ) -> VariableValue {
        self.nodes += 1;
        let Some(ty) = ty else {
            return Self::leaf(name, None, &src, Ok("void".into()));
        };
        let type_name = Some(self.type_name(ty, 0));
        let Some((offset, entry)) = self.strip(ty) else {
            return Self::leaf(name, type_name, &src, Ok("?".into()));
        };
        let size = self.type_size(ty, 0);
        match entry.tag() {
            gimli::DW_TAG_base_type => {
                // `uint8_t` is an unsigned char to DWARF: a number, not a
                // character, unless the declared type says char.
                let as_char = type_name.as_deref().is_some_and(|t| t.contains("char"));
                let value = self.base_value(&entry, &src, size.unwrap_or(0), as_char);
                Self::leaf(name, type_name, &src, value)
            }
            gimli::DW_TAG_pointer_type
            | gimli::DW_TAG_reference_type
            | gimli::DW_TAG_rvalue_reference_type => {
                let raw = src
                    .read(self.mem, size.unwrap_or(4) as usize)
                    .map(|b| le(&b));
                let value = raw.map(|a| self.pointer_text(&entry, a));
                let mut node = Self::leaf(name, type_name, &src, value);
                // A pointer to a struct, union or array shows its target's
                // members, as a debugger's tree does.
                if let (Ok(addr), Some(target)) = (raw, self.type_of(&entry)) {
                    if addr != 0 && depth < MAX_DEPTH && self.is_aggregate(target) {
                        let pointee =
                            self.value(String::new(), Some(target), Src::Mem(addr), depth + 1);
                        node.children = pointee.children;
                    }
                }
                node
            }
            gimli::DW_TAG_enumeration_type => {
                let value = self.enum_value(offset, &entry, &src, size.unwrap_or(4));
                Self::leaf(name, type_name, &src, value)
            }
            gimli::DW_TAG_array_type => self.array(name, type_name, offset, &entry, src, depth),
            gimli::DW_TAG_structure_type | gimli::DW_TAG_class_type | gimli::DW_TAG_union_type => {
                self.aggregate(name, type_name, offset, src, depth)
            }
            gimli::DW_TAG_subroutine_type => Self::leaf(name, type_name, &src, Ok("fn".into())),
            gimli::DW_TAG_unspecified_type => Self::leaf(name, type_name, &src, Ok("void".into())),
            _ => {
                let value = src
                    .read(self.mem, size.unwrap_or(4).min(8) as usize)
                    .map(|b| format!("{:#x}", le(&b)));
                Self::leaf(name, type_name, &src, value)
            }
        }
    }

    fn is_aggregate(&self, ty: UnitOffset) -> bool {
        self.strip(ty).is_some_and(|(_, e)| {
            matches!(
                e.tag(),
                gimli::DW_TAG_structure_type
                    | gimli::DW_TAG_class_type
                    | gimli::DW_TAG_union_type
                    | gimli::DW_TAG_array_type
            )
        })
    }

    fn base_value(
        &self,
        entry: &Entry<'_, '_>,
        src: &Src,
        size: u64,
        as_char: bool,
    ) -> Result<String, Missing> {
        let encoding = match entry.attr_value(gimli::DW_AT_encoding) {
            Ok(Some(AttributeValue::Encoding(e))) => e,
            _ => gimli::DW_ATE_unsigned,
        };
        if size == 0 {
            return Ok("()".into());
        }
        let n = size as usize;
        let bytes = src.read(self.mem, n)?;
        if n > 8 {
            // u128 / i128.
            let mut buf = [0u8; 16];
            let m = n.min(16);
            buf[..m].copy_from_slice(&bytes[..m]);
            return Ok(match encoding {
                gimli::DW_ATE_signed => i128::from_le_bytes(buf).to_string(),
                _ => u128::from_le_bytes(buf).to_string(),
            });
        }
        let raw = le(&bytes);
        Ok(match encoding {
            gimli::DW_ATE_boolean => match raw {
                0 => "false".into(),
                1 => "true".into(),
                v => v.to_string(),
            },
            gimli::DW_ATE_float if n == 4 => format!("{}", f32::from_bits(raw as u32)),
            gimli::DW_ATE_float if n == 8 => format!("{}", f64::from_bits(raw)),
            gimli::DW_ATE_signed => sign_extend(raw, n).to_string(),
            gimli::DW_ATE_signed_char if as_char => {
                let v = sign_extend(raw, n);
                format!("{v} {}", char_text(raw as u32))
            }
            gimli::DW_ATE_signed_char => sign_extend(raw, n).to_string(),
            gimli::DW_ATE_unsigned_char if as_char && n == 1 => {
                format!("{raw} {}", char_text(raw as u32))
            }
            gimli::DW_ATE_UTF => char_text(raw as u32),
            _ => raw.to_string(),
        })
    }

    /// `0x…`, and for a C `char *` the string it points at.
    fn pointer_text(&self, entry: &Entry<'_, '_>, addr: u64) -> String {
        let width = self.address_size() as usize * 2;
        let text = format!("{addr:#0w$x}", w = width + 2);
        if self.rust || addr == 0 {
            return text;
        }
        let is_char = self
            .type_of(entry)
            .and_then(|t| self.strip(t))
            .is_some_and(|(_, e)| {
                e.tag() == gimli::DW_TAG_base_type
                    && udata(&e, gimli::DW_AT_byte_size) == Some(1)
                    && matches!(
                        e.attr_value(gimli::DW_AT_encoding),
                        Ok(Some(AttributeValue::Encoding(
                            gimli::DW_ATE_signed_char | gimli::DW_ATE_unsigned_char
                        )))
                    )
            });
        if !is_char {
            return text;
        }
        match self.c_string(addr) {
            Some(s) => format!("{text} {s}"),
            None => text,
        }
    }

    fn c_string(&self, addr: u64) -> Option<String> {
        let mut bytes = Vec::new();
        for i in 0..MAX_STRING as u64 {
            let mut b = [0u8];
            if !self.mem.read_memory(addr + i, &mut b) {
                return None;
            }
            if b[0] == 0 {
                return Some(format!("{:?}", String::from_utf8_lossy(&bytes)));
            }
            bytes.push(b[0]);
        }
        Some(format!("{:?}…", String::from_utf8_lossy(&bytes)))
    }

    fn enum_value(
        &self,
        offset: UnitOffset,
        entry: &Entry<'_, '_>,
        src: &Src,
        size: u64,
    ) -> Result<String, Missing> {
        let n = (size as usize).clamp(1, 8);
        let raw = le(&src.read(self.mem, n)?);
        let signed = self
            .type_of(entry)
            .and_then(|t| self.strip(t))
            .is_some_and(|(_, e)| {
                matches!(
                    e.attr_value(gimli::DW_AT_encoding),
                    Ok(Some(AttributeValue::Encoding(gimli::DW_ATE_signed)))
                )
            });
        let mask = if n >= 8 { !0u64 } else { (1u64 << (n * 8)) - 1 };
        if let Ok(mut tree) = self.unit.entries_tree(Some(offset)) {
            if let Ok(root) = tree.root() {
                let mut children = root.children();
                while let Ok(Some(child)) = children.next() {
                    let e = child.entry();
                    if e.tag() != gimli::DW_TAG_enumerator {
                        continue;
                    }
                    let v = match e.attr_value(gimli::DW_AT_const_value) {
                        Ok(Some(AttributeValue::Sdata(v))) => v as u64,
                        Ok(Some(v)) => v.udata_value().unwrap_or(u64::MAX),
                        _ => continue,
                    };
                    if v & mask == raw {
                        if let Some(name) = self.name_of(e) {
                            return Ok(name);
                        }
                    }
                }
            }
        }
        Ok(if signed {
            sign_extend(raw, n).to_string()
        } else {
            raw.to_string()
        })
    }

    fn array(
        &mut self,
        name: String,
        type_name: Option<String>,
        offset: UnitOffset,
        entry: &Entry<'_, '_>,
        src: Src,
        depth: usize,
    ) -> VariableValue {
        let elem = self.type_of(entry);
        let dims = self.dimensions(offset);
        let elem_size = elem.and_then(|t| self.type_size(t, 0)).unwrap_or(0);
        let stride = udata(entry, gimli::DW_AT_byte_stride).unwrap_or(elem_size);
        self.array_dims(name, type_name, elem, &dims, stride, src, depth)
    }

    #[allow(clippy::too_many_arguments)]
    fn array_dims(
        &mut self,
        name: String,
        type_name: Option<String>,
        elem: Option<UnitOffset>,
        dims: &[Option<u64>],
        stride: u64,
        src: Src,
        depth: usize,
    ) -> VariableValue {
        let count = dims.first().copied().flatten().unwrap_or(0);
        // The size of one entry of the first dimension.
        let inner: u64 = dims
            .iter()
            .skip(1)
            .map(|d| d.unwrap_or(0))
            .product::<u64>()
            .saturating_mul(stride);
        let mut children = Vec::new();
        if depth < MAX_DEPTH {
            for i in 0..count.min(MAX_ARRAY_CHILDREN) {
                if self.nodes >= MAX_NODES {
                    break;
                }
                let child = src.at(i * inner);
                let child_name = format!("[{i}]");
                children.push(if dims.len() > 1 {
                    self.nodes += 1;
                    let tn = elem.map(|e| self.type_name(e, 0)).map(|e| {
                        let rest: String = dims[1..]
                            .iter()
                            .map(|d| {
                                let d = d.map(|n| n.to_string()).unwrap_or_default();
                                if self.rust {
                                    format!("; {d}]")
                                } else {
                                    format!("[{d}]")
                                }
                            })
                            .collect();
                        if self.rust {
                            format!("{}{e}{rest}", "[".repeat(dims.len() - 1))
                        } else {
                            format!("{e}{rest}")
                        }
                    });
                    self.array_dims(child_name, tn, elem, &dims[1..], stride, child, depth + 1)
                } else {
                    self.value(child_name, elem, child, depth + 1)
                });
            }
        }
        let available = children.is_empty() || children.iter().any(|c| c.available);
        VariableValue {
            name,
            value: type_name.clone().unwrap_or_else(|| format!("[{count}]")),
            type_name,
            available,
            address: src.address(),
            children,
        }
    }

    /// The members of a struct, union or variant type.
    fn members(&self, offset: UnitOffset) -> (Vec<Member>, Option<UnitOffset>) {
        let mut out = Vec::new();
        let mut variant_part = None;
        let Ok(mut tree) = self.unit.entries_tree(Some(offset)) else {
            return (out, None);
        };
        let Ok(root) = tree.root() else {
            return (out, None);
        };
        let mut children = root.children();
        while let Ok(Some(child)) = children.next() {
            let e = child.entry();
            match e.tag() {
                gimli::DW_TAG_member => {
                    // A static member has no storage in the value.
                    if matches!(
                        e.attr_value(gimli::DW_AT_external),
                        Ok(Some(AttributeValue::Flag(true)))
                    ) {
                        continue;
                    }
                    let offset = match e.attr_value(gimli::DW_AT_data_member_location) {
                        Ok(Some(AttributeValue::Exprloc(expr))) => {
                            let mut ops = expr.operations(self.unit.encoding());
                            match ops.next() {
                                Ok(Some(gimli::Operation::PlusConstant { value })) => value,
                                _ => 0,
                            }
                        }
                        Ok(Some(v)) => v.udata_value().unwrap_or(0),
                        _ => 0,
                    };
                    let bits = udata(e, gimli::DW_AT_bit_size).map(|size| {
                        if let Some(b) = udata(e, gimli::DW_AT_data_bit_offset) {
                            (b, size)
                        } else {
                            // DWARF 2/3 counts from the storage's most
                            // significant bit.
                            let storage = udata(e, gimli::DW_AT_byte_size).unwrap_or(4);
                            let from_msb = udata(e, gimli::DW_AT_bit_offset).unwrap_or(0);
                            ((storage * 8).saturating_sub(from_msb + size), size)
                        }
                    });
                    out.push(Member {
                        name: self.name_of(e).unwrap_or_else(|| "?".into()),
                        ty: self.type_of(e),
                        offset,
                        bits,
                    });
                }
                gimli::DW_TAG_variant_part => variant_part = Some(e.offset()),
                _ => {}
            }
        }
        (out, variant_part)
    }

    fn aggregate(
        &mut self,
        name: String,
        type_name: Option<String>,
        offset: UnitOffset,
        src: Src,
        depth: usize,
    ) -> VariableValue {
        let (members, variant_part) = self.members(offset);
        if let Some(part) = variant_part {
            return self.rust_enum(name, type_name, part, &members, src, depth);
        }
        let short = type_name
            .as_deref()
            .map(short_name)
            .unwrap_or_default()
            .to_string();
        // `&str`: show the text.
        if self.rust && type_name.as_deref() == Some("&str") {
            if let Some(text) = self.rust_str(&members, &src) {
                let children = self.member_values(&members, &src, depth);
                return VariableValue {
                    name,
                    type_name,
                    value: text,
                    available: true,
                    address: src.address(),
                    children,
                };
            }
        }
        let children = self.member_values(&members, &src, depth);
        let summary = summary(&short, &children);
        let available = children.is_empty() || children.iter().any(|c| c.available);
        VariableValue {
            name,
            type_name,
            value: if available {
                summary
            } else {
                children[0].value.clone()
            },
            available,
            address: src.address(),
            children,
        }
    }

    fn member_values(&mut self, members: &[Member], src: &Src, depth: usize) -> Vec<VariableValue> {
        let mut children = Vec::new();
        if depth >= MAX_DEPTH {
            return children;
        }
        for m in members {
            if self.nodes >= MAX_NODES {
                break;
            }
            let child_src = src.at(m.offset);
            let v = match m.bits {
                Some((bit_offset, bit_size)) => self.bit_field(m, child_src, bit_offset, bit_size),
                None => self.value(m.name.clone(), m.ty, child_src, depth + 1),
            };
            children.push(v);
        }
        children
    }

    fn bit_field(&mut self, m: &Member, src: Src, bit_offset: u64, bit_size: u64) -> VariableValue {
        self.nodes += 1;
        let type_name = m.ty.map(|t| self.type_name(t, 0));
        let first = bit_offset / 8;
        let last = (bit_offset + bit_size).div_ceil(8);
        let value = src
            .at(first)
            .read(self.mem, (last - first).clamp(1, 8) as usize)
            .map(|b| {
                let raw = le(&b) >> (bit_offset % 8);
                let mask = if bit_size >= 64 {
                    !0
                } else {
                    (1u64 << bit_size) - 1
                };
                let v = raw & mask;
                let signed = m.ty.and_then(|t| self.strip(t)).is_some_and(|(_, e)| {
                    matches!(
                        e.attr_value(gimli::DW_AT_encoding),
                        Ok(Some(AttributeValue::Encoding(gimli::DW_ATE_signed)))
                    )
                });
                if signed && bit_size < 64 && v >> (bit_size - 1) & 1 == 1 {
                    ((v | !mask) as i64).to_string()
                } else {
                    v.to_string()
                }
            });
        Self::leaf(m.name.clone(), type_name, &Src::Bytes(Vec::new()), value)
    }

    fn rust_str(&self, members: &[Member], src: &Src) -> Option<String> {
        let ptr = members.iter().find(|m| m.name == "data_ptr")?;
        let len = members.iter().find(|m| m.name == "length")?;
        let a = self.address_size() as usize;
        let addr = le(&src.at(ptr.offset).read(self.mem, a).ok()?);
        let n = le(&src.at(len.offset).read(self.mem, a).ok()?) as usize;
        let mut buf = vec![0u8; n.min(MAX_STRING)];
        if !self.mem.read_memory(addr, &mut buf) {
            return None;
        }
        let text = format!("{:?}", String::from_utf8_lossy(&buf));
        Some(if n > MAX_STRING {
            format!("{text}…")
        } else {
            text
        })
    }

    /// A Rust enum with data: pick the variant by the discriminant.
    fn rust_enum(
        &mut self,
        name: String,
        type_name: Option<String>,
        part: UnitOffset,
        members: &[Member],
        src: Src,
        depth: usize,
    ) -> VariableValue {
        let Some(part_entry) = self.entry(part) else {
            return Self::leaf(name, type_name, &src, Ok("?".into()));
        };
        // The discriminant member, which DW_AT_discr names.
        let discr = part_entry
            .attr_value(gimli::DW_AT_discr)
            .ok()
            .flatten()
            .and_then(|v| unit_ref(self.unit, v))
            .and_then(|o| {
                let e = self.entry(o)?;
                let offset = match e.attr_value(gimli::DW_AT_data_member_location) {
                    Ok(Some(v)) => v.udata_value().unwrap_or(0),
                    _ => 0,
                };
                let size = self.type_of(&e).and_then(|t| self.type_size(t, 0))?;
                Some((offset, size))
            });
        let discr_value = match discr {
            Some((offset, size)) => {
                match src.at(offset).read(self.mem, (size as usize).clamp(1, 8)) {
                    Ok(b) => Some((le(&b), size)),
                    Err(why) => {
                        return VariableValue {
                            name,
                            type_name,
                            value: why.text().into(),
                            available: false,
                            address: src.address(),
                            children: Vec::new(),
                        }
                    }
                }
            }
            None => None,
        };
        // (discr_value, member) per variant; `None` is the default variant.
        let mut variants: Vec<(Option<u64>, Member)> = Vec::new();
        if let Ok(mut tree) = self.unit.entries_tree(Some(part)) {
            if let Ok(root) = tree.root() {
                let mut children = root.children();
                while let Ok(Some(child)) = children.next() {
                    if child.entry().tag() != gimli::DW_TAG_variant {
                        continue;
                    }
                    let value = match child.entry().attr_value(gimli::DW_AT_discr_value) {
                        Ok(Some(AttributeValue::Sdata(v))) => Some(v as u64),
                        Ok(Some(v)) => v.udata_value(),
                        _ => None,
                    };
                    let mut inner = child.children();
                    while let Ok(Some(m)) = inner.next() {
                        let e = m.entry();
                        if e.tag() != gimli::DW_TAG_member {
                            continue;
                        }
                        let offset = match e.attr_value(gimli::DW_AT_data_member_location) {
                            Ok(Some(v)) => v.udata_value().unwrap_or(0),
                            _ => 0,
                        };
                        variants.push((
                            value,
                            Member {
                                name: self.name_of(e).unwrap_or_else(|| "?".into()),
                                ty: self.type_of(e),
                                offset,
                                bits: None,
                            },
                        ));
                        break;
                    }
                }
            }
        }
        let chosen = match discr_value {
            Some((v, size)) => {
                let mask = if size >= 8 {
                    !0u64
                } else {
                    (1u64 << (size * 8)) - 1
                };
                variants
                    .iter()
                    .find(|(d, _)| d.is_some_and(|d| d & mask == v))
                    .or_else(|| variants.iter().find(|(d, _)| d.is_none()))
            }
            None if variants.len() == 1 => variants.first(),
            None => None,
        };
        let Some((_, variant)) = chosen else {
            // No variant matches: the type name and the raw bytes.
            let raw = discr_value
                .map(|(v, _)| {
                    format!(
                        "{} {{ discriminant: {v} }}",
                        short_name(type_name.as_deref().unwrap_or("enum"))
                    )
                })
                .unwrap_or_else(|| type_name.clone().unwrap_or_default());
            let children = self.member_values(members, &src, depth);
            return VariableValue {
                name,
                type_name,
                value: raw,
                available: true,
                address: src.address(),
                children,
            };
        };
        let fields = match variant.ty {
            Some(t) if depth < MAX_DEPTH => {
                let (fields, _) = self.members(t);
                let src = src.at(variant.offset);
                self.member_values(&fields, &src, depth)
            }
            _ => Vec::new(),
        };
        let value = summary(&variant.name, &fields);
        VariableValue {
            name,
            type_name,
            value,
            available: true,
            address: src.address(),
            children: fields,
        }
    }
}

fn char_text(c: u32) -> String {
    match char::from_u32(c) {
        Some(ch) => format!("{ch:?}"),
        None => format!("\\u{{{c:x}}}"),
    }
}

/// `a::b::Game<T>` -> `Game<T>`.
fn short_name(name: &str) -> &str {
    let generic = name.find('<').unwrap_or(name.len());
    match name[..generic].rfind("::") {
        Some(i) => &name[i + 2..],
        None => name,
    }
}

/// A short struct summary, as VS Code shows one: `Game { paddle: 37,
/// score: 2, … }`, `Some(5)`, `(1, 2)`; `name` alone without fields.
fn summary(name: &str, children: &[VariableValue]) -> String {
    const SHOWN: usize = 3;
    if children.is_empty() {
        return if name.is_empty() {
            "{}".into()
        } else {
            name.to_string()
        };
    }
    let short = |c: &VariableValue| -> String {
        if c.children.is_empty() || c.value.len() <= 24 {
            c.value.clone()
        } else {
            "{…}".into()
        }
    };
    let tuple = children.iter().all(|c| c.name.starts_with("__"));
    let field = |c: &VariableValue| {
        if tuple {
            short(c)
        } else {
            format!("{}: {}", c.name, short(c))
        }
    };
    // Scalars first: `Game { paddle_x: 42, … }` says more than an array's type.
    let mut shown: Vec<&VariableValue> = if tuple {
        children.iter().take(SHOWN).collect()
    } else {
        children
            .iter()
            .filter(|c| c.children.is_empty())
            .take(SHOWN)
            .collect()
    };
    if shown.is_empty() {
        shown = children.iter().take(SHOWN).collect();
    }
    let parts: Vec<String> = shown.iter().map(|c| field(c)).collect();
    let more = if children.len() > shown.len() {
        ", …"
    } else {
        ""
    };
    if tuple {
        format!("{name}({}{more})", parts.join(", "))
    } else if name.is_empty() {
        format!("{{ {}{more} }}", parts.join(", "))
    } else {
        format!("{name} {{ {}{more} }}", parts.join(", "))
    }
}
