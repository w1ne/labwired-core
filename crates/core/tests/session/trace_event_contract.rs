use labwired_core::{emit_trace_event, SimulationObserver};
use labwired_hw_trace::TraceEvent;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct EventRecorder {
    events: Mutex<Vec<TraceEvent>>,
}

impl SimulationObserver for EventRecorder {
    fn on_trace_event(&self, event: TraceEvent) {
        self.events.lock().unwrap().push(event);
    }
}

#[test]
fn simulation_observer_accepts_shared_trace_events() {
    let recorder = EventRecorder::default();

    recorder.on_trace_event(TraceEvent::InstructionRetired {
        pc: 0x0800_0000,
        opcode: 0xBF00,
    });

    assert_eq!(
        recorder.events.lock().unwrap().as_slice(),
        &[TraceEvent::InstructionRetired {
            pc: 0x0800_0000,
            opcode: 0xBF00,
        }]
    );
}

#[test]
fn emit_trace_event_fans_out_to_all_observers() {
    let first = Arc::new(EventRecorder::default());
    let second = Arc::new(EventRecorder::default());
    let observers: Vec<Arc<dyn SimulationObserver>> = vec![first.clone(), second.clone()];

    emit_trace_event(
        &observers,
        TraceEvent::MemoryWrite {
            addr: 0x2000_0000,
            old: 0x12,
            new: 0x34,
        },
    );

    let expected = vec![TraceEvent::MemoryWrite {
        addr: 0x2000_0000,
        old: 0x12,
        new: 0x34,
    }];
    assert_eq!(*first.events.lock().unwrap(), expected);
    assert_eq!(*second.events.lock().unwrap(), expected);
}
