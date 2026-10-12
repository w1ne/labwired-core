// SPDX-License-Identifier: MIT
// Baseline value contracts; real Rust execution is qualified by CoreCI.

use svd_ingestor::{parse_svd, process_peripheral};

#[test]
fn derived_interrupts_override_base_without_changing_register_goldens() {
    let make = |reverse: bool| {
        let base_irqs = if reverse {
            "<interrupt><name>Z</name><value>3</value></interrupt><interrupt><name>DUP</name><value>2</value></interrupt>"
        } else {
            "<interrupt><name>DUP</name><value>2</value></interrupt><interrupt><name>Z</name><value>3</value></interrupt>"
        };
        format!(
            r#"<device schemaVersion="1.3"><name>ORDER</name><version>1</version>
        <description>Order fixture</description><addressUnitBits>8</addressUnitBits><width>32</width>
        <size>32</size><access>read-write</access><resetValue>0</resetValue><peripherals>
        <peripheral><name>BASE</name><baseAddress>0x40000000</baseAddress>{base_irqs}
        <registers><register><name>CR</name><addressOffset>0</addressOffset></register></registers></peripheral>
        <peripheral derivedFrom="BASE"><name>DERIVED</name><baseAddress>0x40001000</baseAddress>
        <interrupt><name>A</name><value>1</value></interrupt><interrupt><name>DUP</name><value>9</value></interrupt>
        <registers><register><name>SR</name><addressOffset>4</addressOffset><access>read-only</access></register></registers>
        </peripheral><peripheral><name>EMPTY</name><baseAddress>0x40002000</baseAddress></peripheral>
        </peripherals></device>"#
        )
    };
    for reverse in [false, true] {
        let device = parse_svd(&make(reverse)).unwrap();
        let p = device
            .peripherals
            .iter()
            .find(|p| p.name == "DERIVED")
            .unwrap();
        let d = process_peripheral(&device, p).unwrap();
        let m = d.interrupts.as_ref().unwrap();
        assert_eq!(m.get("A"), Some(&1));
        assert_eq!(m.get("DUP"), Some(&9));
        assert_eq!(m.get("Z"), Some(&3));
        assert_eq!(
            d.registers
                .iter()
                .map(|r| (
                    r.id.as_str(),
                    r.address_offset,
                    r.size,
                    access_label(&r.access),
                    r.reset_value,
                    r.fields.len()
                ))
                .collect::<Vec<_>>(),
            vec![("CR", 0, 32, "rw", 0, 0), ("SR", 4, 32, "ro", 0, 0)]
        );
        let empty = device
            .peripherals
            .iter()
            .find(|p| p.name == "EMPTY")
            .unwrap();
        assert!(process_peripheral(&device, empty)
            .unwrap()
            .interrupts
            .is_none());
    }
}

fn access_label(access: &labwired_config::Access) -> &'static str {
    match access {
        labwired_config::Access::ReadWrite => "rw",
        labwired_config::Access::ReadOnly => "ro",
        labwired_config::Access::WriteOnly => "wo",
    }
}
