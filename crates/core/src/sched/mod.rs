// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Event-driven peripheral scheduler skeleton (Phase 2B.1, issue #192).
//!
//! See `/tmp/event_scheduler_design.md` for the signed-off design. This
//! module ships the API surface only — no peripheral opts in here.

pub mod clock;
pub mod event_scheduler;
pub mod result;

pub use clock::{ClockDomain, ClockGraph};
pub use event_scheduler::{
    EventScheduler, ScheduledEvent, SchedulerStats, SimCycle, SUBSYSTEM_PERIPHERAL_IDX,
};
pub use result::EventResult;
