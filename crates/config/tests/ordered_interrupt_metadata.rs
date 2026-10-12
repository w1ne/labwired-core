// SPDX-License-Identifier: MIT
// Baseline value contracts; real Rust execution is qualified by CoreCI.

use labwired_config::PeripheralDescriptor;
use labwired_ir::{IrInterrupt, IrPeripheral};

const HEADER: &str = "peripheral: ORDER\nversion: v1\nregisters: []\n";
fn parse(tail: &str) -> PeripheralDescriptor {
    PeripheralDescriptor::from_yaml(&format!("{HEADER}{tail}")).unwrap()
}

#[test]
fn typed_yaml_duplicate_interrupt_last_value_wins() {
    let d = parse("interrupts:\n  DUP: 1\n  OTHER: 4\n  DUP: 7\n");
    let m = d.interrupts.unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m.get("DUP"), Some(&7));
    assert_eq!(m.get("OTHER"), Some(&4));
}

#[test]
fn optional_empty_and_unknown_fields_keep_existing_contract() {
    assert!(parse("").interrupts.is_none());
    assert!(parse("interrupts: null\n").interrupts.is_none());
    assert!(parse("interrupts: {}\n").interrupts.unwrap().is_empty());
    assert!(parse("future_metadata: ignored\n").interrupts.is_none());
}

#[test]
fn malformed_interrupts_still_fail() {
    for tail in [
        "interrupts: {A: -1}\n",
        "interrupts: {A: 4294967296}\n",
        "interrupts: {A: text}\n",
        "interrupts: [1, 2]\n",
    ] {
        assert!(
            PeripheralDescriptor::from_yaml(&format!("{HEADER}{tail}")).is_err(),
            "{tail}"
        );
    }
    for missing in [
        "version: v1\nregisters: []\n",
        "peripheral: A\nregisters: []\n",
        "peripheral: A\nversion: v1\n",
    ] {
        assert!(PeripheralDescriptor::from_yaml(missing).is_err());
    }
}

#[test]
fn ir_duplicate_and_empty_interrupt_semantics_stay_exact() {
    let make = |interrupts| IrPeripheral {
        name: "IR".into(),
        base_address: 0,
        description: None,
        registers: vec![],
        interrupts,
        timing: vec![],
    };
    let irq = |name: &str, value| IrInterrupt {
        name: name.into(),
        value,
        description: None,
    };
    let d = PeripheralDescriptor::from(make(vec![
        irq("Z", 2),
        irq("DUP", 3),
        irq("A", 1),
        irq("DUP", 9),
    ]));
    assert_eq!(d.version, "ir-v1");
    assert!(d.registers.is_empty());
    assert!(d.timing.is_none());
    let m = d.interrupts.unwrap();
    assert_eq!(m.get("DUP"), Some(&9));
    assert!(PeripheralDescriptor::from(make(vec![]))
        .interrupts
        .is_none());
}
