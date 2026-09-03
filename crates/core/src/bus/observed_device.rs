// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! One registry for the off-chip models the bus holds ONLY so something can
//! read them back.
//!
//! # What these are
//!
//! A WS2812 strip, a hobby servo, a STEP/DIR stepper, an H-bridge channel, a
//! 4-phase unipolar stepper and a parallel ILI9341 panel are all driven the
//! same way: a GPIO (or LEDC duty) observer holds an `Arc` clone of the model
//! and decodes edges into it. The bus itself never ticks them, never routes to
//! them, and never reads them — it holds a second `Arc` clone purely so
//! `inspect`, the canvas and the CLI can ask the model what it currently shows.
//!
//! # Why they are not six fields
//!
//! They used to be. `SystemBus` carried
//!
//! ```text
//! pub ws2812:            Vec<Arc<Ws2812>>,
//! pub servos:            Vec<Arc<Servo>>,
//! pub step_dir_motors:   Vec<Arc<StepDirMotor>>,
//! pub h_bridge_motors:   Vec<Arc<HBridgeMotor>>,
//! pub ili9341_parallel:  Vec<Arc<Ili9341Parallel>>,
//! pub unipolar_steppers: Vec<Arc<UnipolarStepper>>,
//! ```
//!
//! — six public fields naming six concrete off-chip parts, each with a matching
//! arm in [`for_each_bus_resident_device`], each with a line in every
//! `SystemBus` struct literal in the tree. That is the C-1 ledger row in one
//! screen: the *shape of the engine* changed every time a readback-only part
//! was added, and none of those six fields carried a single byte of behaviour
//! the other five did not.
//!
//! Now there is one [`SystemBus::observed`] list of [`ObservedDevice`] and one
//! arm. Adding a seventh readback-only part is an `impl` next to the model, not
//! an edit to the bus.
//!
//! # What it deliberately is NOT
//!
//! It is not "every device the bus holds". The HC-SR04, the TM1637, the HX711,
//! the direct 7-segment, the analog sources and the three CAN node kinds each
//! keep their own field, because the bus really does drive each of them and
//! each drive is a different shape — an armed echo deadline, a two-wire
//! protocol clock, a combinational resample, a stimulus walk, a scheduled frame
//! injection. Collapsing those would replace six honest fields with one
//! dishonest trait. This registry covers exactly the models the bus holds and
//! does nothing with.
//!
//! [`for_each_bus_resident_device`]: super::SystemBus

use super::SystemBus;
use std::any::Any;
use std::sync::Arc;

/// An off-chip model the bus holds only for readback.
///
/// Implement it beside the model, next to its `attach`. The three identity
/// methods are what the attached-device walk needs to report the part under the
/// name its author gave it; `as_any` / `as_arc_any` are what a typed reader
/// needs to get the concrete model back out (see [`SystemBus::observed_of`]).
pub trait ObservedDevice: std::fmt::Debug + Send + Sync {
    /// The `external_devices:` id this model was built from — the author's own
    /// text, which is what the inspect join matches on.
    fn manifest_id(&self) -> &str;

    /// This model's own id, when ONE declaration built several models.
    ///
    /// An H-bridge board is the case: one `external_devices:` entry builds two
    /// independent channels (`<id>-a`, `<id>-b`). Each reports its own identity
    /// here and both join back to the declaration they came from, so neither is
    /// anonymous and neither claims to be the whole board. `None` — the default
    /// — means the model is the whole of what was declared.
    fn model_id(&self) -> Option<&str> {
        None
    }

    /// What this model can show, when it is a display.
    ///
    /// `None` is the honest answer for everything else: a servo and a stepper
    /// have no display surface, and `None` says that rather than promising an
    /// empty screen.
    fn evidence(&self) -> Option<&dyn crate::inspect::DeviceEvidence> {
        None
    }

    /// Borrowed concrete-type escape hatch (see [`SystemBus::observed_of`]).
    fn as_any(&self) -> &dyn Any;

    /// Owned twin of [`as_any`](Self::as_any), for the readers that need to
    /// keep the model alive past the borrow of the bus (see
    /// [`SystemBus::observed_arcs_of`]). Every impl is `{ self }`.
    fn as_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

impl SystemBus {
    /// Iterate the readback-only off-chip models of concrete type `T`.
    ///
    /// This is the typed replacement for indexing `bus.servos` /
    /// `bus.ili9341_parallel` / … : the bus no longer names the type, so the
    /// reader does.
    pub fn observed_of<T: ObservedDevice + 'static>(&self) -> impl Iterator<Item = &T> {
        self.observed
            .iter()
            .filter_map(|d| d.as_any().downcast_ref::<T>())
    }

    /// Owned twin of [`Self::observed_of`], for a reader that needs the model
    /// to outlive its borrow of the bus.
    pub fn observed_arcs_of<T: ObservedDevice + 'static>(
        &self,
    ) -> impl Iterator<Item = Arc<T>> + '_ {
        self.observed
            .iter()
            .filter_map(|d| Arc::clone(d).as_arc_any().downcast::<T>().ok())
    }

    /// Register a readback-only off-chip model. Called by each model's own
    /// `attach`; the bus learns nothing about what it just took.
    pub fn observe_device<T: ObservedDevice + 'static>(&mut self, device: Arc<T>) {
        self.observed.push(device);
    }
}
