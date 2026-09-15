// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Errors a [`crate::session::Session`] reports.

/// Why a session operation did not complete.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The engine cannot do this here. The one error for an honest gap: never
    /// a silent no-op.
    #[error("not supported: {what}")]
    NotSupported { what: &'static str },
    /// `expect` spent its whole virtual-time budget without a match, or the
    /// machine stopped (`halted`) before one could appear.
    #[error(
        "expect timed out after {virtual_seconds}s waiting for /{pattern}/{}; last output: {tail:?}",
        if *halted { " (machine halted)" } else { "" }
    )]
    ExpectTimeout {
        pattern: String,
        virtual_seconds: f64,
        /// The last 200 characters of the UART transcript.
        tail: String,
        /// The machine stopped executing before the budget ran out.
        halted: bool,
    },
    #[error("unknown uart {0:?}")]
    UnknownUart(String),
    /// The firmware ELF defines no symbol with this name.
    #[error("unknown symbol {0:?}")]
    UnknownSymbol(String),
    /// No `board_io` input binding has this id.
    #[error("no input board_io binding {0:?}")]
    UnknownPin(String),
    /// No peripheral on the bus has this name.
    #[error("unknown peripheral {0:?}")]
    UnknownPeripheral(String),
    /// The named peripheral exists but is not a CAN controller.
    #[error("peripheral {0:?} is not a CAN controller")]
    NotACanController(String),
    /// The CAN controller refused the frame, as silicon would.
    #[error("CAN controller {bus:?} did not receive the frame: {reason}")]
    CanRejected {
        bus: String,
        reason: crate::network::CanRxRejection,
    },
    /// The frame is not a valid CAN or CAN-FD frame.
    #[error("invalid CAN frame: {0}")]
    InvalidCanFrame(String),
    #[error(transparent)]
    Sim(#[from] crate::SimulationError),
    #[error(transparent)]
    Input(#[from] crate::sim_input::SimInputError),
    #[error("{0}")]
    Other(String),
}

pub type SessionResult<T> = Result<T, SessionError>;
