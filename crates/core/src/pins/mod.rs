// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Pins: the one interface between a chip's pads and the world.
//!
//! A pad has two sources of electrical truth:
//!
//! 1. **The chip's own output stage** ([`PadDriver`]), decided by the chip's
//!    registers only: driving low, driving high, or released, plus an optional
//!    internal pull.
//! 2. **The outside world** ([`External`]): whatever the board connects to the
//!    pad (a net, a button, a sensor), which presents a level, or nothing.
//!
//! Every GPIO model implements [`PinPort`]; everything that touches a pad
//! (the world's `gpio_net`, the logic analyzer and its four-state trace, board
//! buttons, EXTI edge routing, co-simulation, the bindings) goes through it.
//! The rule that combines the two sides into what a probe sees is
//! [`resolve`], written once here. See `docs/architecture/pins.md`.
//!
//! The services built on the trait live here too:
//!
//! * [`probe_drive`] / [`own_drive`]: the drive a logic-analyzer pad channel
//!   (`LogicSource::Pad`) and a world net member (`LogicSource::Driver`)
//!   report, from [`PinPort::driver`] and [`PinPort::external`] only.
//! * [`PadWatch`]: push capture. A model stores the watch the machine installs
//!   ([`PinPort::install_watch`]) and brackets each pad mutation with
//!   [`watch_begin`] / [`watch_end`]; the watch snapshots the watched pads
//!   through the trait and pushes what changed. Models write no capture code.
//! * [`PortId`]: the number the bus gives a port when it is attached, which
//!   edge sinks (EXTI) are addressed by.

use crate::logic_capture::{LogicTap, PadDrive};

#[cfg(test)]
mod conformance;

/// What a chip's output stage does with a pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Out {
    /// Released: the output stage does not drive (an input, an analog pad,
    /// an open-drain output holding a 1).
    Off,
    /// Driving low.
    Low,
    /// Driving high.
    High,
}

impl Out {
    /// `Low`/`High` for a driven `level`.
    #[inline]
    pub fn from_level(level: bool) -> Self {
        if level {
            Self::High
        } else {
            Self::Low
        }
    }

    /// The driven level, `None` when released.
    #[inline]
    pub fn level(self) -> Option<bool> {
        match self {
            Self::Off => None,
            Self::Low => Some(false),
            Self::High => Some(true),
        }
    }
}

/// The internal pull resistor on a pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Pull {
    #[default]
    None,
    Up,
    Down,
}

impl Pull {
    /// The level the pull holds an otherwise undriven pad at.
    #[inline]
    pub fn level(self) -> Option<bool> {
        match self {
            Self::None => None,
            Self::Up => Some(true),
            Self::Down => Some(false),
        }
    }

    /// `Some(true)` up, `Some(false)` down, `None` none.
    #[inline]
    pub fn from_up(up: Option<bool>) -> Self {
        match up {
            Some(true) => Self::Up,
            Some(false) => Self::Down,
            None => Self::None,
        }
    }
}

/// What this chip's output stage does to the pad. Register truth only: never
/// includes a level applied from outside ([`External`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PadDriver {
    /// `Off | Low | High`. An open-drain output holding a 1 is `Off`.
    pub out: Out,
    /// The internal pull resistor, whatever `out` is.
    pub pull: Pull,
}

impl PadDriver {
    /// Released, no pull.
    pub const OFF: Self = Self {
        out: Out::Off,
        pull: Pull::None,
    };

    /// Driving `level`, no pull.
    #[inline]
    pub fn drive(level: bool) -> Self {
        Self {
            out: Out::from_level(level),
            pull: Pull::None,
        }
    }

    /// Released with `pull`.
    #[inline]
    pub fn released(pull: Pull) -> Self {
        Self {
            out: Out::Off,
            pull,
        }
    }

    /// The same output stage with `pull`.
    #[inline]
    pub fn with_pull(self, pull: Pull) -> Self {
        Self { pull, ..self }
    }

    /// A push-pull output driving `level`, or an open-drain one, which drives
    /// only a 0 and releases for a 1.
    #[inline]
    pub fn output(level: bool, open_drain: bool) -> Self {
        if open_drain && level {
            Self::OFF
        } else {
            Self::drive(level)
        }
    }
}

/// What the outside world presents to a pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum External {
    /// Nothing connected, or the connection let go.
    #[default]
    Released,
    /// Something drives the wire to this level.
    Level(bool),
}

impl External {
    /// The presented level, `None` when released.
    #[inline]
    pub fn level(self) -> Option<bool> {
        match self {
            Self::Released => None,
            Self::Level(level) => Some(level),
        }
    }
}

/// The firmware-visible input level of a pad before and after
/// [`PinPort::set_external`], so the caller can raise edge events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputChange {
    pub before: bool,
    pub after: bool,
}

impl InputChange {
    /// True when the input moved.
    #[inline]
    pub fn changed(self) -> bool {
        self.before != self.after
    }
}

/// The number the bus gives a GPIO port when it is attached, in the chip's
/// own port numbering (port A = 0). Edge sinks are addressed by it: an STM32
/// EXTI line-source mux compares it against `EXTICRx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortId(pub u8);

impl PortId {
    /// The port number a chip yaml's port id spells: `gpio` followed by one
    /// letter (`gpioa` = 0, `gpiob` = 1, …), case-insensitive. Asked once,
    /// when the bus attaches the port; `None` for any other id, which leaves
    /// the port without edge sinks.
    pub fn from_port_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        let suffix = lower.strip_prefix("gpio")?;
        let &[letter] = suffix.as_bytes() else {
            return None;
        };
        let n = letter.checked_sub(b'a')?;
        (n < 16).then_some(Self(n))
    }
}

/// The one interface between a chip's pads and the world. Implemented by
/// every GPIO model, and by anything else that owns pads.
///
/// Pins are numbered `0..pin_count()` within the port. A pin out of range
/// answers `None` (and `set_external` refuses it).
pub trait PinPort {
    /// Pads this port owns (pins `0..count`).
    fn pin_count(&self) -> u8;

    /// The chip's own drive on `pin`, decoded from registers only, `None`
    /// when the model cannot say (a pad handed to a peripheral signal whose
    /// drive is not published).
    fn driver(&self, pin: u8) -> Option<PadDriver>;

    /// What the outside world currently presents to `pin` (the last
    /// [`set_external`](Self::set_external)). [`External::Released`] for a
    /// pad nothing was ever connected to.
    fn external(&self, pin: u8) -> External;

    /// Present `ext` to the pad's input buffer. The model updates what
    /// firmware reads and returns the input level before and after. `None`:
    /// the pad cannot take input.
    ///
    /// A model brackets this with [`watch_begin`] / [`watch_end`] so a probe
    /// on the pad sees the change.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange>;

    /// The firmware-visible input level of `pin` (`IDR`, `PINx`, `GPIO_IN`).
    fn input(&self, pin: u8) -> Option<bool>;

    /// The level a logic probe clipped to `pin` reads.
    ///
    /// The default is the shared rule: [`resolve`] the driver against the
    /// external, and where that leaves the level undetermined (released, no
    /// pull, nothing presented: a floating pad) the input register. A model
    /// overrides it only where its input register has a documented quirk the
    /// probe must agree with (see `docs/architecture/pins.md`).
    fn level(&self, pin: u8) -> Option<bool> {
        match self.driver(pin) {
            Some(driver) => resolve(driver, self.external(pin))
                .level
                .or_else(|| self.input(pin)),
            None => self.external(pin).level().or_else(|| self.input(pin)),
        }
    }

    /// `pin` joined a wire shared with other chips (a world `gpio_net`).
    /// Peripheral lines routed to the pad switch to their multi-chip mode.
    /// Nothing about the drive changes: [`driver`](Self::driver) never
    /// included the external level. Default: nothing to do.
    fn join_wire(&mut self, _pin: u8) {}

    /// Install (`Some`) or clear (`None`) the push-capture watch. Returns
    /// `true` when this port reports its watched pads itself (brackets every
    /// pad mutation with [`watch_begin`] / [`watch_end`]); `false` (the
    /// default) leaves them on the machine's per-cycle poll.
    fn install_watch(&mut self, _watch: Option<PadWatch>) -> bool {
        false
    }

    /// Lend the installed watch out for one bracket half ([`watch_begin`],
    /// [`watch_end`]). Default: none installed.
    fn take_watch(&mut self) -> Option<PadWatch> {
        None
    }

    /// Return the watch [`take_watch`](Self::take_watch) lent out.
    fn put_watch(&mut self, _watch: PadWatch) {}

    /// A bracketed mutation may have re-routed pads: re-register the watched
    /// channels with the peripheral lines that now drive them. Called by
    /// [`watch_end`]. Default: no routed pads.
    fn routes_changed(&mut self) {}
}

/// A pad's level and drive after [`resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    /// The level on the wire, `None` when nothing determines it (released,
    /// no pull, nothing presented).
    pub level: Option<bool>,
    /// Who determines it.
    pub drive: PadDrive,
}

/// The shared rule: what a pad does given the chip's own output stage and
/// what the outside world presents.
///
/// * An output stage driving `l` with nothing outside: `l`, driven.
/// * Driving `l` while the outside presents `l`: driven; the other level:
///   contention (the level reported is the chip's own, as every model's
///   input register already does).
/// * Released, the outside presents `e`: `e`, driven (from outside).
/// * Released, nothing outside: the internal pull's weak level, or nothing
///   (high-Z).
pub fn resolve(driver: PadDriver, ext: External) -> Resolved {
    match (driver.out.level(), ext.level()) {
        (Some(own), None) => Resolved {
            level: Some(own),
            drive: PadDrive::Driven,
        },
        (Some(own), Some(outside)) => Resolved {
            level: Some(own),
            drive: if own == outside {
                PadDrive::Driven
            } else {
                PadDrive::Contention
            },
        },
        (None, Some(outside)) => Resolved {
            level: Some(outside),
            drive: PadDrive::Driven,
        },
        (None, None) => match driver.pull {
            Pull::Up => Resolved {
                level: Some(true),
                drive: PadDrive::PullUp,
            },
            Pull::Down => Resolved {
                level: Some(false),
                drive: PadDrive::PullDown,
            },
            Pull::None => Resolved {
                level: None,
                drive: PadDrive::HighZ,
            },
        },
    }
}

/// The drive a probe clipped to the pad reports (a `LogicSource::Pad`
/// channel, the four-state pin trace): [`resolve`] of driver and external,
/// with an internal pull reported as high-Z (`h`/`l` belong to net members,
/// see [`own_drive`]). A pad whose driver is unknown reports an external
/// level as driven and otherwise nothing.
pub fn probe_drive(port: &dyn PinPort, pin: u8) -> Option<PadDrive> {
    let ext = port.external(pin);
    match port.driver(pin) {
        Some(driver) => Some(match resolve(driver, ext).drive {
            PadDrive::PullUp | PadDrive::PullDown => PadDrive::HighZ,
            drive => drive,
        }),
        None => ext.level().map(|_| PadDrive::Driven),
    }
}

/// The drive of the chip's own output stage alone (a `LogicSource::Driver`
/// channel, a world `gpio_net` member): [`resolve`] of the driver against
/// nothing outside, so an internal pull shows as a weak level.
pub fn own_drive(port: &dyn PinPort, pin: u8) -> Option<PadDrive> {
    port.driver(pin)
        .map(|driver| resolve(driver, External::Released).drive)
}

/// Which side of a pad a watched channel reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadView {
    /// The pad as a probe sees it: [`PinPort::level`] and [`probe_drive`].
    Probe,
    /// The chip's own output stage: [`PinPort::level`] and [`own_drive`].
    Driver,
}

impl PadView {
    /// The drive this view reports for `pin`.
    #[inline]
    pub fn drive(self, port: &dyn PinPort, pin: u8) -> Option<PadDrive> {
        match self {
            Self::Probe => probe_drive(port, pin),
            Self::Driver => own_drive(port, pin),
        }
    }
}

/// One watched pad of a [`PadWatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchedPad {
    pub pin: u8,
    /// Logic-analyzer channel.
    pub ch: u32,
    pub view: PadView,
}

/// Push capture for one port: the machine's [`LogicTap`] and the watched
/// pads. A model stores it ([`PinPort::install_watch`]) and brackets each pad
/// mutation (register write, external input, a re-route from another block)
/// with [`watch_begin`] / [`watch_end`]. The watch reads every watched pad
/// through the trait before and after, and pushes each one whose level, or
/// known drive, changed. Unwatched pads cost nothing.
#[derive(Debug, Clone)]
pub struct PadWatch {
    tap: LogicTap,
    pads: Vec<WatchedPad>,
    /// `(pin, channel)` of every watched pad, for registering channels with
    /// peripheral lines (`PadRoutes::sync_taps`).
    pairs: Vec<(u8, u32)>,
    /// Level and drive of each pad at the last [`Self::snapshot`].
    before: Vec<(Option<bool>, Option<PadDrive>)>,
}

impl PadWatch {
    /// `None` for an empty watch set (nothing to report).
    pub fn new(tap: &LogicTap, pads: &[WatchedPad]) -> Option<Self> {
        (!pads.is_empty()).then(|| Self {
            tap: tap.clone(),
            pads: pads.to_vec(),
            pairs: pads.iter().map(|p| (p.pin, p.ch)).collect(),
            before: vec![(None, None); pads.len()],
        })
    }

    /// A watch of probe channels only, from `(pin, channel)` pairs.
    pub fn probes(tap: &LogicTap, pairs: &[(u8, u32)]) -> Option<Self> {
        let pads: Vec<WatchedPad> = pairs
            .iter()
            .map(|&(pin, ch)| WatchedPad {
                pin,
                ch,
                view: PadView::Probe,
            })
            .collect();
        Self::new(tap, &pads)
    }

    /// The machine's tap.
    pub fn tap(&self) -> &LogicTap {
        &self.tap
    }

    /// `(pin, channel)` of every watched pad.
    pub fn pairs(&self) -> &[(u8, u32)] {
        &self.pairs
    }

    /// The watched pads.
    pub fn pads(&self) -> &[WatchedPad] {
        &self.pads
    }

    /// Record each watched pad's level and drive.
    pub fn snapshot(&mut self, port: &dyn PinPort) {
        for (k, pad) in self.pads.iter().enumerate() {
            self.before[k] = (port.level(pad.pin), pad.view.drive(port, pad.pin));
        }
    }

    /// Push every watched pad whose level, or known drive, changed since the
    /// last [`Self::snapshot`]. A pad whose level is unknown now reports
    /// nothing (the poll path keeps the last known level too).
    pub fn report(&self, port: &dyn PinPort) {
        for (k, pad) in self.pads.iter().enumerate() {
            let Some(level) = port.level(pad.pin) else {
                continue;
            };
            let drive = pad.view.drive(port, pad.pin);
            let (before_level, before_drive) = self.before[k];
            match drive {
                // Report the drive with the level whenever EITHER moved: an
                // open-drain release keeps the pulled level and only the
                // drive changes.
                Some(d) if before_level != Some(level) || before_drive != drive => {
                    self.tap.push_with_drive(pad.ch, level, d);
                }
                _ if before_level != Some(level) => self.tap.push(pad.ch, level),
                _ => {}
            }
        }
    }
}

/// First half of a push-capture bracket: snapshot the port's watched pads.
/// No-op (one call) while no watch is installed.
#[inline]
pub fn watch_begin(port: &mut dyn PinPort) {
    if let Some(mut watch) = port.take_watch() {
        watch.snapshot(&*port);
        port.put_watch(watch);
    }
}

/// Second half: push what changed since [`watch_begin`], then let the port
/// re-register its watched channels with the lines that now drive them.
#[inline]
pub fn watch_end(port: &mut dyn PinPort) {
    if let Some(watch) = port.take_watch() {
        watch.report(&*port);
        port.put_watch(watch);
        port.routes_changed();
    }
}

/// [`PinPort::set_external`] for a caller that only knows a level (the
/// legacy `set_gpio_input` contract): `true` when the pad took it.
#[inline]
pub fn set_level(port: &mut dyn PinPort, pin: u8, level: bool) -> bool {
    port.set_external(pin, External::Level(level)).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_covers_every_side() {
        let low = PadDriver::drive(false);
        let high = PadDriver::drive(true);
        let up = PadDriver::released(Pull::Up);
        let down = PadDriver::released(Pull::Down);
        let off = PadDriver::OFF;
        let r = |d, e| resolve(d, e);
        assert_eq!(
            r(high, External::Released),
            Resolved {
                level: Some(true),
                drive: PadDrive::Driven
            }
        );
        assert_eq!(r(low, External::Level(false)).drive, PadDrive::Driven);
        assert_eq!(
            r(low, External::Level(true)),
            Resolved {
                level: Some(false),
                drive: PadDrive::Contention
            }
        );
        assert_eq!(
            r(off, External::Level(true)),
            Resolved {
                level: Some(true),
                drive: PadDrive::Driven
            }
        );
        // An external level beats a pull.
        assert_eq!(r(up, External::Level(false)).level, Some(false));
        assert_eq!(
            r(up, External::Released),
            Resolved {
                level: Some(true),
                drive: PadDrive::PullUp
            }
        );
        assert_eq!(r(down, External::Released).level, Some(false));
        assert_eq!(
            r(off, External::Released),
            Resolved {
                level: None,
                drive: PadDrive::HighZ
            }
        );
    }

    #[test]
    fn open_drain_one_is_released() {
        assert_eq!(PadDriver::output(true, true).out, Out::Off);
        assert_eq!(PadDriver::output(false, true).out, Out::Low);
        assert_eq!(PadDriver::output(true, false).out, Out::High);
    }

    #[test]
    fn port_ids_follow_the_port_letter() {
        assert_eq!(PortId::from_port_name("gpioa"), Some(PortId(0)));
        assert_eq!(PortId::from_port_name("GPIOC"), Some(PortId(2)));
        assert_eq!(PortId::from_port_name("gpio"), None);
        assert_eq!(PortId::from_port_name("gpio0"), None);
        assert_eq!(PortId::from_port_name("portb"), None);
        assert_eq!(PortId::from_port_name("gpioab"), None);
    }
}
