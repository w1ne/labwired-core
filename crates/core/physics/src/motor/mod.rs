// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

mod bldc;
mod brushed;
mod encoder;
mod shaft;

pub use bldc::{
    phase_back_emf_shapes, trapezoidal_back_emf, BldcFaults, BldcMotor, BldcMotorParams,
    BldcMotorSnapshot, GatePair, HallSensors, Inverter, InverterCommand, InverterFault,
    InverterResolution, Phase, PhaseTerminal,
};
pub use brushed::{
    BrushedDcMotor, BrushedMotorParams, BrushedMotorSnapshot, HBridgeCommand, HBridgeState,
    MotorFaults,
};
pub use encoder::{EncoderPins, QuadratureEncoder};
pub use shaft::{ModelError, Shaft, ShaftParams, ShaftSnapshot};
