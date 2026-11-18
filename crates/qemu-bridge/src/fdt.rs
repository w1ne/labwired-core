// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Minimal flattened device tree (DTB, version 17) reader.
//!
//! Reads the structure block into an arena of nodes with raw properties.
//! Interpretation (cells, ranges, interrupts) lives in [`crate::dt`].

use std::collections::HashMap;

const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

#[derive(Debug, Clone)]
pub struct Node {
    /// Unit name, e.g. `i2c@7e804000`; empty for the root.
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub props: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone)]
pub struct Fdt {
    pub nodes: Vec<Node>,
    phandles: HashMap<u32, usize>,
}

fn be32(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
        .ok_or_else(|| format!("DTB truncated at {off:#x}"))
}

impl Fdt {
    pub fn parse(b: &[u8]) -> Result<Self, String> {
        if be32(b, 0)? != FDT_MAGIC {
            return Err("not a DTB (bad magic)".into());
        }
        let off_struct = be32(b, 8)? as usize;
        let off_strings = be32(b, 12)? as usize;
        let version = be32(b, 20)?;
        if version < 16 {
            return Err(format!("DTB version {version} not supported"));
        }
        let string_at = |off: usize| -> Result<String, String> {
            let s = b
                .get(off_strings + off..)
                .ok_or("string offset out of range")?;
            let end = s
                .iter()
                .position(|&c| c == 0)
                .ok_or("unterminated string")?;
            Ok(String::from_utf8_lossy(&s[..end]).into_owned())
        };
        let mut nodes: Vec<Node> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut p = off_struct;
        loop {
            let tok = be32(b, p)?;
            p += 4;
            match tok {
                FDT_BEGIN_NODE => {
                    let s = b.get(p..).ok_or("node name out of range")?;
                    let end = s.iter().position(|&c| c == 0).ok_or("unterminated name")?;
                    let name = String::from_utf8_lossy(&s[..end]).into_owned();
                    p = (p + end + 1 + 3) & !3;
                    let idx = nodes.len();
                    let parent = stack.last().copied();
                    nodes.push(Node {
                        name,
                        parent,
                        children: Vec::new(),
                        props: Vec::new(),
                    });
                    if let Some(par) = parent {
                        nodes[par].children.push(idx);
                    }
                    stack.push(idx);
                }
                FDT_END_NODE => {
                    stack.pop().ok_or("unbalanced END_NODE")?;
                }
                FDT_PROP => {
                    let len = be32(b, p)? as usize;
                    let nameoff = be32(b, p + 4)? as usize;
                    p += 8;
                    let val = b.get(p..p + len).ok_or("property out of range")?.to_vec();
                    p = (p + len + 3) & !3;
                    let cur = *stack.last().ok_or("property outside a node")?;
                    nodes[cur].props.push((string_at(nameoff)?, val));
                }
                FDT_NOP => {}
                FDT_END => break,
                t => return Err(format!("bad DTB token {t:#x} at {:#x}", p - 4)),
            }
        }
        if nodes.is_empty() {
            return Err("DTB has no root node".into());
        }
        let mut fdt = Fdt {
            nodes,
            phandles: HashMap::new(),
        };
        for i in 0..fdt.nodes.len() {
            for key in ["phandle", "linux,phandle"] {
                if let Some(v) = fdt.prop_u32(i, key) {
                    fdt.phandles.insert(v, i);
                }
            }
        }
        Ok(fdt)
    }

    pub fn root(&self) -> usize {
        0
    }

    pub fn prop(&self, node: usize, name: &str) -> Option<&[u8]> {
        self.nodes[node]
            .props
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
    }

    pub fn prop_u32(&self, node: usize, name: &str) -> Option<u32> {
        self.prop(node, name)
            .filter(|v| v.len() >= 4)
            .map(|v| u32::from_be_bytes(v[0..4].try_into().unwrap()))
    }

    pub fn prop_cells(&self, node: usize, name: &str) -> Option<Vec<u32>> {
        self.prop(node, name).map(|v| {
            v.chunks_exact(4)
                .map(|c| u32::from_be_bytes(c.try_into().unwrap()))
                .collect()
        })
    }

    pub fn prop_str(&self, node: usize, name: &str) -> Option<String> {
        self.prop(node, name).map(|v| {
            let end = v.iter().position(|&c| c == 0).unwrap_or(v.len());
            String::from_utf8_lossy(&v[..end]).into_owned()
        })
    }

    pub fn prop_strs(&self, node: usize, name: &str) -> Vec<String> {
        self.prop(node, name)
            .map(|v| {
                v.split(|&c| c == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn by_phandle(&self, ph: u32) -> Option<usize> {
        self.phandles.get(&ph).copied()
    }

    pub fn path(&self, node: usize) -> String {
        let mut parts = Vec::new();
        let mut n = Some(node);
        while let Some(i) = n {
            if self.nodes[i].parent.is_some() {
                parts.push(self.nodes[i].name.as_str());
            }
            n = self.nodes[i].parent;
        }
        if parts.is_empty() {
            return "/".into();
        }
        parts.reverse();
        format!("/{}", parts.join("/"))
    }

    pub fn find(&self, path: &str) -> Option<usize> {
        let mut cur = self.root();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            cur = *self.nodes[cur].children.iter().find(|&&c| {
                let n = &self.nodes[c].name;
                n == part || n.split('@').next() == Some(part)
            })?;
        }
        Some(cur)
    }

    /// Depth-first order, root first.
    pub fn walk(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack = vec![self.root()];
        while let Some(n) = stack.pop() {
            out.push(n);
            stack.extend(self.nodes[n].children.iter().rev());
        }
        out
    }
}
