// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The electrical half of a world `gpio_net`: given what each member pad
//! drives over time, what level does the wire carry, when does each member
//! see it, and where do drivers fight.
//!
//! No machines in here. The [`World`](crate::world::World) feeds
//! [`GpioNet::apply`] the members' drive changes in world-time order and
//! hands the resulting [`Delivery`]s to the member machines at the exact
//! cycle. Keeping the resolution pure is what makes it testable on its own
//! and independent of node order and round size.
//!
//! Resolution, for the drives present at one instant. Every member is
//! released (`Z`), driving 0 or 1 (strong), or held by its own internal
//! pull-up or pull-down (weak); the net's `pull` is one more weak source:
//!
//! * any member driving 0 -> 0, else any member driving 1 -> 1;
//! * else the weak sources (members' internal pulls and the net's `pull`):
//!   all up -> 1, all down -> 0;
//! * up and down together is a resistor divider, a pull conflict: it is
//!   reported, and the wire takes the net's own `pull` (the board resistor,
//!   normally far stronger than a chip's 30-50 kOhm internal one) or, when
//!   the net has none, 0 (like contention);
//! * no driver and no pull -> the net floats: it reads 0 and is flagged.
//!
//! Members driving 0 and 1 together are in contention: the net resolves to 0
//! (a low-side driver usually wins on silicon) and the fight is reported,
//! never dropped.

use labwired_config::GpioNetPull;
use serde::Serialize;

/// What one member's own pad does to the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Own {
    /// Released (input, open-drain high) with no internal pull: drives nothing.
    Z,
    /// Output stage driving 0 (strong).
    Low,
    /// Output stage driving 1 (strong).
    High,
    /// Released, with the chip's internal pull-up on: a weak 1.
    PullUp,
    /// Released, with the chip's internal pull-down on: a weak 0.
    PullDown,
}

/// A level change the wire carries to every member at `t_ps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivery {
    pub t_ps: u64,
    pub level: bool,
}

/// Diagnostic code for two members driving opposite levels.
pub const GPIO_NET_CONTENTION: &str = "GPIO_NET_CONTENTION";
/// Diagnostic code for a net nothing drives and nothing pulls.
pub const GPIO_NET_FLOATING: &str = "GPIO_NET_FLOATING";
/// Diagnostic code for weak pulls to opposite rails with nothing driving
/// (a resistor divider: the wire sits between the rails).
pub const GPIO_NET_PULL_CONFLICT: &str = "GPIO_NET_PULL_CONFLICT";

/// One diagnostic on a net, on the net's own timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetDiagnostic {
    /// `GPIO_NET_CONTENTION`, `GPIO_NET_FLOATING` or `GPIO_NET_PULL_CONFLICT`.
    pub code: &'static str,
    /// World time the condition began, ps.
    pub t_ps: u64,
    /// World time it ended, ps; `None` while it lasts.
    pub end_ps: Option<u64>,
    /// Every member's drive when it began.
    pub members: Vec<MemberDrive>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberDrive {
    pub node: String,
    pub peripheral: String,
    pub pin: u8,
    pub drive: Own,
    /// The member node's own cycle at that instant.
    pub cycle: u64,
}

/// Most diagnostics kept per net (a net that fights for ever must not grow
/// the report without bound); the counters keep counting.
pub const MAX_NET_DIAGNOSTICS: usize = 256;

/// One member pad's identity and its current drive.
#[derive(Debug, Clone)]
pub struct NetMember {
    pub node: String,
    pub peripheral: String,
    pub pin: u8,
    /// The member node's clock, Hz (to put diagnostics on its cycle axis).
    pub hz: u64,
    own: Own,
    /// Index into `GpioNet::deliveries` of the next one to hand this member.
    next: usize,
}

/// Counters and diagnostics of one net, for a run report.
#[derive(Debug, Clone, Serialize)]
pub struct GpioNetReport {
    pub name: String,
    pub pull: &'static str,
    pub latency_ns: f64,
    /// Level the wire carries now.
    pub level: bool,
    /// Level changes so far.
    pub edges: u64,
    pub contention_events: u64,
    pub floating_events: u64,
    pub pull_conflict_events: u64,
    pub members: Vec<NetMemberReport>,
    pub diagnostics: Vec<NetDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetMemberReport {
    pub node: String,
    pub peripheral: String,
    pub pin: u8,
    pub drive: Own,
}

/// One net: members, pull, latency and the history that follows from them.
#[derive(Debug, Clone)]
pub struct GpioNet {
    pub name: String,
    pub pull: GpioNetPull,
    pub latency_ps: u64,
    members: Vec<NetMember>,
    level: bool,
    contention: bool,
    floating: bool,
    pull_conflict: bool,
    deliveries: Vec<Delivery>,
    /// Deliveries dropped from the front of `deliveries` (all members past).
    base: usize,
    edges: u64,
    contention_events: u64,
    floating_events: u64,
    pull_conflict_events: u64,
    diagnostics: Vec<NetDiagnostic>,
}

impl GpioNet {
    pub fn new(
        name: String,
        pull: GpioNetPull,
        latency_ps: u64,
        members: Vec<(String, String, u8, u64)>,
    ) -> Self {
        Self {
            name,
            pull,
            latency_ps,
            members: members
                .into_iter()
                .map(|(node, peripheral, pin, hz)| NetMember {
                    node,
                    peripheral,
                    pin,
                    hz,
                    own: Own::Z,
                    next: 0,
                })
                .collect(),
            level: false,
            contention: false,
            floating: false,
            pull_conflict: false,
            deliveries: Vec::new(),
            base: 0,
            edges: 0,
            contention_events: 0,
            floating_events: 0,
            pull_conflict_events: 0,
            diagnostics: Vec::new(),
        }
    }

    pub fn members(&self) -> &[NetMember] {
        &self.members
    }

    /// The level the wire carries now (after the changes applied so far).
    pub fn level(&self) -> bool {
        self.level
    }

    /// The resolution rule, on its own (see the module docs).
    pub fn resolve(drives: impl Iterator<Item = Own>, pull: GpioNetPull) -> Resolved {
        let (mut low, mut high, mut up, mut down) = (false, false, false, false);
        for d in drives {
            match d {
                Own::Low => low = true,
                Own::High => high = true,
                Own::PullUp => up = true,
                Own::PullDown => down = true,
                Own::Z => {}
            }
        }
        let quiet = Resolved {
            level: false,
            contention: false,
            floating: false,
            pull_conflict: false,
        };
        if low || high {
            // A strong driver: weak pulls do not matter.
            return Resolved {
                level: !low,
                contention: low && high,
                ..quiet
            };
        }
        match pull {
            GpioNetPull::Up => up = true,
            GpioNetPull::Down => down = true,
            GpioNetPull::None => {}
        }
        match (up, down) {
            (true, false) => Resolved {
                level: true,
                ..quiet
            },
            (false, true) => quiet,
            // A divider: the board's own resistor wins, else read 0.
            (true, true) => Resolved {
                level: matches!(pull, GpioNetPull::Up),
                pull_conflict: true,
                ..quiet
            },
            (false, false) => Resolved {
                floating: true,
                ..quiet
            },
        }
    }

    /// Set every member's drive at time 0 (before any change) and return the
    /// level the wire starts at. A net that starts floating, fighting or
    /// with conflicting pulls is recorded from time 0.
    pub fn init(&mut self, initial: &[Own]) -> bool {
        for (m, o) in self.members.iter_mut().zip(initial) {
            m.own = *o;
        }
        let r = Self::resolve(self.members.iter().map(|m| m.own), self.pull);
        self.level = r.level;
        self.contention = r.contention;
        self.floating = r.floating;
        self.pull_conflict = r.pull_conflict;
        for (on, code) in [
            (r.contention, GPIO_NET_CONTENTION),
            (r.floating, GPIO_NET_FLOATING),
            (r.pull_conflict, GPIO_NET_PULL_CONFLICT),
        ] {
            if on {
                *self.counter(code) += 1;
                let members = self.snapshot(0);
                self.push_diag(NetDiagnostic {
                    code,
                    t_ps: 0,
                    end_ps: None,
                    members,
                });
            }
        }
        self.level
    }

    fn counter(&mut self, code: &str) -> &mut u64 {
        match code {
            GPIO_NET_CONTENTION => &mut self.contention_events,
            GPIO_NET_FLOATING => &mut self.floating_events,
            _ => &mut self.pull_conflict_events,
        }
    }

    /// Open or close the diagnostic `code` when its condition changes.
    fn track(&mut self, code: &'static str, was: bool, now: bool, t_ps: u64) {
        if was == now {
            return;
        }
        if now {
            *self.counter(code) += 1;
            let members = self.snapshot(t_ps);
            self.push_diag(NetDiagnostic {
                code,
                t_ps,
                end_ps: None,
                members,
            });
        } else {
            self.close_diag(code, t_ps);
        }
    }

    fn snapshot(&self, t_ps: u64) -> Vec<MemberDrive> {
        self.members
            .iter()
            .map(|m| MemberDrive {
                node: m.node.clone(),
                peripheral: m.peripheral.clone(),
                pin: m.pin,
                drive: m.own,
                cycle: crate::network::timed_uart::ps_to_cycles_ceil(t_ps, m.hz),
            })
            .collect()
    }

    /// Apply the drive changes that happen at world time `t_ps` (member
    /// index, new drive), resolve once, and queue the level change for
    /// delivery at `t_ps + latency` if the wire moved. Callers apply changes
    /// in nondecreasing `t_ps`.
    pub fn apply(&mut self, t_ps: u64, changes: &[(usize, Own)]) {
        for &(i, own) in changes {
            self.members[i].own = own;
        }
        let r = Self::resolve(self.members.iter().map(|m| m.own), self.pull);
        self.track(GPIO_NET_CONTENTION, self.contention, r.contention, t_ps);
        self.contention = r.contention;
        self.track(GPIO_NET_FLOATING, self.floating, r.floating, t_ps);
        self.floating = r.floating;
        self.track(
            GPIO_NET_PULL_CONFLICT,
            self.pull_conflict,
            r.pull_conflict,
            t_ps,
        );
        self.pull_conflict = r.pull_conflict;
        if r.level != self.level {
            self.level = r.level;
            self.edges += 1;
            self.deliveries.push(Delivery {
                t_ps: t_ps + self.latency_ps,
                level: r.level,
            });
        }
    }

    fn push_diag(&mut self, d: NetDiagnostic) {
        if self.diagnostics.len() < MAX_NET_DIAGNOSTICS {
            self.diagnostics.push(d);
        }
    }

    fn close_diag(&mut self, code: &str, t_ps: u64) {
        if let Some(d) = self
            .diagnostics
            .iter_mut()
            .rev()
            .find(|d| d.code == code && d.end_ps.is_none())
        {
            d.end_ps = Some(t_ps);
        }
    }

    /// The next level change member `m` has not been handed yet.
    pub fn next_due(&self, m: usize) -> Option<Delivery> {
        self.deliveries
            .get(self.members[m].next - self.base)
            .copied()
    }

    /// Mark member `m`'s next delivery handed over.
    pub fn consume(&mut self, m: usize) {
        self.members[m].next += 1;
        let min = self.members.iter().map(|m| m.next).min().unwrap_or(0);
        if min - self.base >= 1024 {
            self.deliveries.drain(..min - self.base);
            self.base = min;
        }
    }

    pub fn report(&self) -> GpioNetReport {
        GpioNetReport {
            name: self.name.clone(),
            pull: match self.pull {
                GpioNetPull::None => "none",
                GpioNetPull::Up => "up",
                GpioNetPull::Down => "down",
            },
            latency_ns: self.latency_ps as f64 / 1000.0,
            level: self.level,
            edges: self.edges,
            contention_events: self.contention_events,
            floating_events: self.floating_events,
            pull_conflict_events: self.pull_conflict_events,
            members: self
                .members
                .iter()
                .map(|m| NetMemberReport {
                    node: m.node.clone(),
                    peripheral: m.peripheral.clone(),
                    pin: m.pin,
                    drive: m.own,
                })
                .collect(),
            diagnostics: self.diagnostics.clone(),
        }
    }
}

/// The wire's state for one set of drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub level: bool,
    pub contention: bool,
    pub floating: bool,
    /// Weak pulls to both rails and no driver.
    pub pull_conflict: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(pull: GpioNetPull) -> GpioNet {
        GpioNet::new(
            "n".into(),
            pull,
            100_000,
            vec![
                ("a".into(), "gpioa".into(), 1, 72_000_000),
                ("b".into(), "portd".into(), 2, 16_000_000),
            ],
        )
    }

    #[test]
    fn low_beats_high_and_both_is_contention() {
        let r = GpioNet::resolve([Own::High, Own::Low].into_iter(), GpioNetPull::None);
        assert_eq!(
            r,
            Resolved {
                level: false,
                contention: true,
                floating: false,
                pull_conflict: false,
            }
        );
        let r = GpioNet::resolve([Own::High, Own::Z].into_iter(), GpioNetPull::Down);
        assert!(r.level && !r.contention);
        let r = GpioNet::resolve([Own::Low, Own::Z].into_iter(), GpioNetPull::Up);
        assert!(!r.level && !r.contention);
    }

    #[test]
    fn pull_decides_when_nobody_drives_and_none_floats() {
        let up = GpioNet::resolve([Own::Z, Own::Z].into_iter(), GpioNetPull::Up);
        assert!(up.level && !up.floating);
        let down = GpioNet::resolve([Own::Z, Own::Z].into_iter(), GpioNetPull::Down);
        assert!(!down.level && !down.floating);
        let none = GpioNet::resolve([Own::Z, Own::Z].into_iter(), GpioNetPull::None);
        assert!(!none.level && none.floating);
    }

    #[test]
    fn open_drain_with_pull_up_is_a_wired_and() {
        let mut n = net(GpioNetPull::Up);
        assert!(n.init(&[Own::Z, Own::Z]));
        n.apply(1_000_000, &[(0, Own::Low)]);
        n.apply(2_000_000, &[(1, Own::Low)]);
        n.apply(3_000_000, &[(0, Own::Z)]); // b still holds it low
        n.apply(4_000_000, &[(1, Own::Z)]);
        let levels: Vec<_> = (0..)
            .map_while(|_| {
                let d = n.next_due(0)?;
                n.consume(0);
                Some((d.t_ps, d.level))
            })
            .collect();
        assert_eq!(levels, vec![(1_100_000, false), (4_100_000, true)]);
        assert_eq!(n.report().contention_events, 0);
    }

    #[test]
    fn contention_is_reported_with_its_time_and_resolves_low() {
        let mut n = net(GpioNetPull::None);
        n.init(&[Own::Low, Own::Z]);
        n.apply(5_000_000, &[(0, Own::High), (1, Own::Low)]);
        let rep = n.report();
        assert_eq!(rep.contention_events, 1);
        let d = rep
            .diagnostics
            .iter()
            .find(|d| d.code == GPIO_NET_CONTENTION)
            .unwrap();
        assert_eq!(d.t_ps, 5_000_000);
        assert!(!n.level());
        n.apply(6_000_000, &[(0, Own::Z)]);
        let d = n
            .report()
            .diagnostics
            .into_iter()
            .find(|d| d.code == GPIO_NET_CONTENTION)
            .unwrap();
        assert_eq!(d.end_ps, Some(6_000_000));
    }

    #[test]
    fn a_floating_net_is_flagged_from_the_start() {
        let mut n = net(GpioNetPull::None);
        n.init(&[Own::Z, Own::Z]);
        assert_eq!(n.report().floating_events, 1);
        assert!(!n.level());
    }

    #[test]
    fn an_internal_pull_is_a_weak_level_any_driver_overrides() {
        // The only pull on the net is a member's own pull-up.
        let r = GpioNet::resolve([Own::PullUp, Own::Z].into_iter(), GpioNetPull::None);
        assert!(r.level && !r.floating && !r.pull_conflict);
        let r = GpioNet::resolve([Own::PullDown, Own::Z].into_iter(), GpioNetPull::None);
        assert!(!r.level && !r.floating && !r.pull_conflict);
        // A strong driver beats any pull, internal or external, silently.
        let r = GpioNet::resolve([Own::PullUp, Own::Low].into_iter(), GpioNetPull::Up);
        assert!(!r.level && !r.contention && !r.pull_conflict);
        let r = GpioNet::resolve(
            [Own::PullDown, Own::PullUp, Own::High].into_iter(),
            GpioNetPull::Down,
        );
        assert!(r.level && !r.pull_conflict);
        // Pulls that agree, internal and external, are just that level.
        let r = GpioNet::resolve([Own::PullUp, Own::PullUp].into_iter(), GpioNetPull::Up);
        assert!(r.level && !r.pull_conflict);
    }

    #[test]
    fn opposite_pulls_are_a_conflict_the_board_resistor_decides() {
        // Internal up vs the net's pull-down: the net's own resistor wins.
        let r = GpioNet::resolve([Own::PullUp, Own::Z].into_iter(), GpioNetPull::Down);
        assert_eq!(
            r,
            Resolved {
                level: false,
                contention: false,
                floating: false,
                pull_conflict: true,
            }
        );
        let r = GpioNet::resolve([Own::PullDown, Own::Z].into_iter(), GpioNetPull::Up);
        assert!(r.level && r.pull_conflict);
        // Two chips pulling opposite ways and no net pull: read 0, flagged.
        let r = GpioNet::resolve([Own::PullUp, Own::PullDown].into_iter(), GpioNetPull::None);
        assert!(!r.level && r.pull_conflict && !r.floating);
    }

    #[test]
    fn a_pull_conflict_is_reported_with_its_time() {
        let mut n = net(GpioNetPull::None);
        // a pulls up, b floats: the wire is high from the start.
        assert!(n.init(&[Own::PullUp, Own::Z]));
        n.apply(2_000_000, &[(1, Own::PullDown)]);
        assert!(!n.level());
        n.apply(3_000_000, &[(1, Own::Z)]);
        assert!(n.level());
        let rep = n.report();
        assert_eq!(rep.pull_conflict_events, 1);
        assert_eq!(rep.floating_events, 0);
        let d = rep
            .diagnostics
            .iter()
            .find(|d| d.code == GPIO_NET_PULL_CONFLICT)
            .unwrap();
        assert_eq!((d.t_ps, d.end_ps), (2_000_000, Some(3_000_000)));
        assert_eq!(d.members[0].drive, Own::PullUp);
        assert_eq!(d.members[1].drive, Own::PullDown);
        assert_eq!(rep.members[0].drive, Own::PullUp);
        // high at init, low at 2 us, high again at 3 us
        assert_eq!(rep.edges, 2);
    }

    #[test]
    fn enabling_an_internal_pull_ends_floating() {
        let mut n = net(GpioNetPull::None);
        assert!(!n.init(&[Own::Z, Own::Z]));
        n.apply(1_000_000, &[(0, Own::PullUp)]);
        assert!(n.level());
        let d = n
            .report()
            .diagnostics
            .into_iter()
            .find(|d| d.code == GPIO_NET_FLOATING)
            .unwrap();
        assert_eq!(d.end_ps, Some(1_000_000));
        assert_eq!(
            n.next_due(0).map(|d| (d.t_ps, d.level)),
            Some((1_100_000, true))
        );
    }
}
