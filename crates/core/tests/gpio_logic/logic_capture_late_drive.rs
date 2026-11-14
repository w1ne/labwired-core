// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use labwired_core::logic_capture::{
    attach_logic_states, build_logic_edges_result, LogicCapture, LogicChannelMeta, LogicEdge,
    LogicEdgeBatch, LogicSource, LogicStateBatch, LogicStateEdge, PadDrive, PadEvent, PadState,
};

fn late_drive(push: bool) -> (LogicEdgeBatch, LogicStateBatch) {
    let sources = [Some(LogicSource::pad(0, 0)), Some(LogicSource::pad(0, 1))];
    let mut capture = LogicCapture::new();
    capture.install(&sources, &[None, None], &[push, push]);
    assert_eq!(capture.install_drives(&[None, None]), vec![None, None]);
    // Channel 0's model first reports drive at cycle 10. Channel 1 never
    // reports drive, despite publishing the same observed Boolean levels.
    for (cycle, value, drive) in [
        (5, false, None),
        (10, true, Some(PadDrive::Driven)),
        (20, true, Some(PadDrive::HighZ)),
        (30, false, Some(PadDrive::Driven)),
    ] {
        if push {
            capture.ingest_push(
                &[
                    PadEvent {
                        ch: 1,
                        cycle,
                        value,
                        drive: None,
                    },
                    PadEvent {
                        ch: 0,
                        cycle,
                        value,
                        drive,
                    },
                ],
                cycle,
                cycle,
            );
        } else {
            capture.sample_with_drive(
                cycle,
                |_| Some(value),
                |source| {
                    if source == LogicSource::pad(0, 0) {
                        drive
                    } else {
                        None
                    }
                },
            );
        }
    }
    (capture.read_edges(0), capture.read_states(0))
}

#[test]
fn late_drive_states_preserve_push_poll_and_boolean_parity() {
    let (push_edges, push_states) = late_drive(true);
    let (poll_edges, poll_states) = late_drive(false);
    assert_eq!(push_edges.edges, poll_edges.edges);
    assert_eq!(push_states.edges, poll_states.edges);
    assert_eq!(push_edges.dropped, 0);
    assert_eq!(poll_edges.dropped, 0);
    assert_eq!(push_states.dropped, 0);
    assert_eq!(poll_states.dropped, 0);
    let expected_edges: Vec<_> = [(5, false), (10, true), (30, false)]
        .into_iter()
        .flat_map(|(cycle, value)| [0, 1].map(|ch| LogicEdge { ch, cycle, value }))
        .collect();
    assert_eq!(push_edges.edges, expected_edges);
    assert_eq!(
        push_states.edges,
        vec![
            LogicStateEdge {
                ch: 0,
                cycle: 10,
                state: PadState::High
            },
            LogicStateEdge {
                ch: 0,
                cycle: 20,
                state: PadState::HighZ
            },
            LogicStateEdge {
                ch: 0,
                cycle: 30,
                state: PadState::Low
            },
        ]
    );
}

#[test]
fn export_keeps_late_states_without_inventing_an_initial_state() {
    let metadata: Vec<_> = [0, 1]
        .map(|ch| LogicChannelMeta {
            ch,
            peripheral: "gpio".into(),
            pin: ch as u8,
            initial: None,
        })
        .into();
    let boolean = LogicEdgeBatch {
        cursor: 0,
        dropped: 0,
        edges: vec![],
    };
    let states = LogicStateBatch {
        cursor: 1,
        dropped: 0,
        edges: vec![LogicStateEdge {
            ch: 0,
            cycle: 10,
            state: PadState::High,
        }],
    };
    let mut result = build_logic_edges_result(&metadata, &boolean, 20);
    attach_logic_states(&mut result, &[None, None], &states);
    assert_eq!(result.channels[0].initial_state, None);
    let observed = result.channels[0]
        .states
        .as_ref()
        .expect("late native state is retained");
    assert_eq!(observed.len(), 1);
    assert_eq!((observed[0].cycle, observed[0].state), (10, PadState::High));
    assert_eq!(result.channels[1].initial_state, None);
    assert_eq!(
        result.channels[1].states, None,
        "unsupported channel stays unknown"
    );
}
