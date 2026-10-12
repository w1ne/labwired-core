// SPDX-License-Identifier: MIT
// Ordered assertions: expected compiler RED until the admitted production migration.

use labwired_config::PeripheralDescriptor;
use labwired_ir::{IrInterrupt, IrPeripheral};
use std::collections::BTreeMap;

const HEADER: &str = "peripheral: ORDER\nversion: v1\nregisters: []\n";
fn parse(tail: &str) -> PeripheralDescriptor {
    PeripheralDescriptor::from_yaml(&format!("{HEADER}{tail}")).unwrap()
}

#[test]
fn public_ordered_type_contract() {
    let d = parse("interrupts: {Z: 2, A: 1}\n");
    let _: &Option<BTreeMap<String, u32>> = &d.interrupts;
}

#[test]
fn input_order_does_not_change_serialized_interrupts() {
    let names = [
        "z", "A", "irq10", "irq2", "a", "é", "Ω", "Z", "B", "b", "n", "M", "m", "C", "c", "IRQ",
    ];
    let expected = BTreeMap::from_iter(
        names
            .iter()
            .enumerate()
            .map(|(i, s)| (s.to_string(), i as u32)),
    );
    let mut outputs = Vec::new();
    for shift in 0..names.len() {
        let mut indices: Vec<usize> = (0..names.len()).collect();
        indices.rotate_left(shift);
        if shift % 2 == 1 {
            indices.reverse();
        }
        let mut text = String::from("interrupts:\n");
        for i in indices {
            text.push_str(&format!("  {}: {}\n", names[i], i));
        }
        let d = parse(&text);
        let actual = d.interrupts.as_ref().unwrap();
        assert_eq!(
            actual.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>()
        );
        for (name, number) in &expected {
            assert_eq!(actual.get(name), Some(number));
        }
        outputs.push(serde_yaml::to_string(&d).unwrap());
    }
    assert!(outputs.windows(2).all(|pair| pair[0] == pair[1]));
    let restored = PeripheralDescriptor::from_yaml(&outputs[0]).unwrap();
    assert_eq!(restored.interrupts.as_ref().unwrap(), &expected);
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
    assert_eq!(
        m.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["A", "DUP", "Z"]
    );
    assert_eq!(m.get("DUP"), Some(&9));
    assert!(PeripheralDescriptor::from(make(vec![]))
        .interrupts
        .is_none());
}
