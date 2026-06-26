use labwired_hw_trace::{FaultEffect, TraceEvent};

#[test]
fn trace_events_cover_the_shared_evidence_contract() {
    let retired = TraceEvent::InstructionRetired {
        pc: 0x0800_0124,
        opcode: 0xD001,
    };
    let branch = TraceEvent::BranchEdge {
        src: 0x0800_0124,
        target: 0x0800_0130,
        taken: true,
    };
    let write = TraceEvent::MemoryWrite {
        addr: 0x4002_1000,
        old: 0,
        new: 1,
    };
    let fault = TraceEvent::FaultInjected {
        kind: "missing_clock".to_string(),
        target: "rcc.apb2enr.iopaen".to_string(),
        at_step: 42,
        at_pc: 0x0800_0200,
        effect: FaultEffect::DroppedClockGate,
    };

    assert_eq!(retired.pc(), Some(0x0800_0124));
    assert_eq!(branch.pc(), Some(0x0800_0124));
    assert_eq!(write.pc(), None);
    assert_eq!(fault.pc(), Some(0x0800_0200));
}

#[test]
fn trace_events_are_json_round_trippable() {
    let event = TraceEvent::FaultInjected {
        kind: "register_bit_flip".to_string(),
        target: "gpioa.odr".to_string(),
        at_step: 7,
        at_pc: 0x0800_0042,
        effect: FaultEffect::RegisterBitFlip {
            addr: 0x4002_000c,
            bit: 5,
        },
    };

    let encoded = serde_json::to_string(&event).expect("serialize trace event");
    let decoded: TraceEvent = serde_json::from_str(&encoded).expect("deserialize trace event");

    assert_eq!(decoded, event);
}
