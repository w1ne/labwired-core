// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Location expressions that need no machine state, on the committed Nokia
//! 5110 fixture (Rust release build).

use labwired_loader::{FrameContext, RegisterSet, SymbolProvider, TargetMemory};
use std::path::PathBuf;

/// A target with no readable memory: only constants evaluate.
struct NoMemory;

impl TargetMemory for NoMemory {
    fn read_memory(&self, _addr: u64, _buf: &mut [u8]) -> bool {
        false
    }
}

/// `let mut target = 0;` (main.rs:493) is `DW_OP_consts 0; DW_OP_stack_value`
/// where no echo was measured: an implicit value, shown as `0`, not as
/// "optimized out".
#[test]
fn constant_location_shows_its_value() {
    let elf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/nokia5110-invaders-lab.elf");
    let symbols = SymbolProvider::new(&elf).unwrap();
    let regs = RegisterSet::default();
    let mut constant_at = Vec::new();
    let (start, size, _) = symbols
        .functions()
        .iter()
        .find(|(_, _, name)| name.contains("__cortex_m_rt_main"))
        .cloned()
        .expect("main's symbol");
    for pc in (start..start + size).step_by(2) {
        let ctx = FrameContext {
            pc,
            regs: &regs,
            cfa: None,
            mem: &NoMemory,
            register_size: 4,
        };
        let vars = symbols.variables(&ctx, None);
        if let Some(t) = vars.iter().find(|v| v.name == "target" && v.available) {
            assert_eq!(t.value, "0", "at {pc:#x}");
            assert_eq!(t.type_name.as_deref(), Some("i32"));
            assert_eq!(t.address, None);
            constant_at.push(pc);
        }
        // Without registers or memory, a stack slot is not available, but
        // still listed with its type.
        if let Some(game) = vars.iter().find(|v| v.name == "game") {
            assert!(!game.available);
            assert_eq!(game.type_name.as_deref(), Some("Game"));
        }
    }
    assert!(
        !constant_at.is_empty(),
        "no pc where target is the constant 0"
    );
}
