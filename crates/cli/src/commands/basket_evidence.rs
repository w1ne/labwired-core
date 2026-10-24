//! Independent phase predicates. Unit fixtures are parser controls, not guest proof.
use labwired_core::bus::BusTraceEvent;
use serde::Serialize;
use serde_json::Value;
#[derive(Debug, Clone, Serialize)]
pub(super) struct PhaseProof {
    pub action_cycle: u64,
    pub from_seq: u64,
    pub through_seq: u64,
    pub first_retained_seq: u64,
    pub evicted: u64,
    pub events: Vec<BusTraceEvent>,
    pub peer_before: Vec<String>,
    pub peer_after: Vec<String>,
    pub sensor: Value,
    pub radio: Value,
    pub raw5: f64,
    pub raw3: f64,
    pub effective3: f64,
    pub host_held: bool,
}

pub(super) const PHASE_IDS: [&str; 4] = [
    "stimulus_a",
    "stimulus_b",
    "external_off",
    "external_restored",
];

pub(super) fn validate_phase(index: usize, console: &str, p: &PhaseProof) -> Result<(), String> {
    use labwired_core::bus::bus_trace::{BusDir, BusPayload, I2cSym};
    let require = |ok: bool, reason: &str| if ok { Ok(()) } else { Err(reason.to_owned()) };
    require(index < 4, "unknown phase")?;
    require(
        p.through_seq >= p.from_seq
            && p.first_retained_seq <= p.from_seq + 1
            && p.events.len() as u64 == p.through_seq - p.from_seq
            && p.events
                .iter()
                .enumerate()
                .all(|(i, e)| e.seq == p.from_seq + 1 + i as u64),
        "incomplete actual phase trace interval; absence cannot be proved after eviction",
    )?;
    require(
        p.raw5 == 5. && p.raw3 == 3.3 && !p.host_held,
        "phase requires actual healthy plant rails and running host",
    )?;
    require(
        p.effective3 == if index == 2 { 0. } else { 3.3 },
        "external effective rail mismatch",
    )?;
    let powered = index != 2;
    for meta in [&p.sensor, &p.radio] {
        require(
            meta["powered"].as_bool() == Some(powered),
            "canonical device supply state mismatch",
        )?;
        require(
            meta["supply_generation"].as_u64().is_some_and(|n| n > 0),
            "missing actual supply generation",
        )?;
    }
    require(
        p.radio["command_bytes"].as_u64() == Some(0)
            && p.radio["reply_bytes"].as_u64() == Some(0)
            && p.radio["mock_connected"].as_bool() == Some(false),
        "canonical radio has stale queue or connection",
    )?;
    let traffic: Vec<_> = p
        .events
        .iter()
        .filter(|e| e.bus == "i2c3" || e.bus == "uart4")
        .collect();
    // READY is followed by the actual prepared 100 ms guest wait. The host may
    // observe its final console byte up to one 4096-cycle window late.
    require(
        traffic.iter().all(|e| {
            e.cycle > p.action_cycle && e.cycle >= p.action_cycle.saturating_add(2_400_000 - 4096)
        }),
        "host actuation did not precede real guest transfer and prepared wait",
    )?;
    let i2c: Vec<_> = traffic
        .iter()
        .filter(|e| e.bus == "i2c3")
        .map(|e| &e.payload)
        .collect();
    let uart = |dir| {
        traffic
            .iter()
            .filter_map(|e| match &e.payload {
                BusPayload::Uart { direction, byte } if e.bus == "uart4" && *direction == dir => {
                    Some(*byte)
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let actual_lines: Vec<_> = console
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| {
            [
                "LW_BASKET_IO ",
                "LW_BASKET_DEVID ",
                "LW_BASKET_AXES ",
                "LW_BASKET_UART ",
            ]
            .iter()
            .any(|prefix| l.starts_with(prefix))
        })
        .collect();
    if !powered {
        require(i2c.is_empty(), "off rail reached inner I2C device")?;
        require(
            uart(BusDir::Tx) == b"AT\r\n" && uart(BusDir::Rx).is_empty(),
            "off UART must attempt AT without enqueueing any RX",
        )?;
        require(
            p.peer_before == p.peer_after,
            "off rail changed accepted canonical peer commands",
        )?;
        require(
            actual_lines
                == [
                    "LW_BASKET_IO kind=i2c_rdwr ret=-6 count=2 errno=6",
                    "LW_BASKET_UART command=AT written=4 received=0 errno=0 reply=",
                ],
            "actual Linux off-rail ENXIO/termios timeout contract failed",
        )?;
        return Ok(());
    }
    let axes = if index == 0 {
        [256, -128, 64]
    } else {
        [-64, 128, -256]
    };
    let inputs = if index == 0 {
        [1., -0.5, 0.25]
    } else {
        [-0.25, 0.5, -1.]
    };
    for (axis, expected) in ["x", "y", "z"].into_iter().zip(inputs) {
        require(
            p.sensor["inputs"][axis].as_f64() == Some(expected),
            "same-instance component input mismatch",
        )?;
    }
    for (register, expected) in [
        "DEVID",
        "POWER_CTL",
        "DATA_FORMAT",
        "DATAX0",
        "DATAY0",
        "DATAZ0",
    ]
    .into_iter()
    .zip([229, 8, 8, axes[0], axes[1], axes[2]])
    {
        require(
            p.sensor["registers"][register].as_i64() == Some(expected),
            "same-instance sensor register mismatch",
        )?;
    }
    let a = I2cSym::AddrWrite;
    let r = I2cSym::AddrRead;
    let d = I2cSym::Data;
    let mut symbols = vec![
        (a, 166),
        (d, 45),
        (d, 8),
        (a, 166),
        (d, 49),
        (d, 8),
        (a, 166),
        (d, 0),
        (r, 167),
        (d, 229),
        (a, 166),
        (d, 50),
        (r, 167),
    ];
    for axis in axes {
        for byte in (axis as i16).to_le_bytes() {
            symbols.push((d, byte));
        }
    }
    let expected: Vec<_> = symbols
        .into_iter()
        .map(|(kind, byte)| BusPayload::I2c {
            kind,
            byte,
            ack: true,
        })
        .collect();
    require(
        i2c.len() == expected.len() && i2c.iter().zip(&expected).all(|(a, b)| *a == b),
        "actual canonical I2C wire bytes/count/ack mismatch",
    )?;
    let phases: Value = serde_json::from_str(include_str!(
        "../../../../examples/luckfox-pico-mini-a-linux/basket/phases.json"
    ))
    .map_err(|e| e.to_string())?;
    let mut lines = vec![
        "LW_BASKET_IO kind=i2c_rdwr ret=1 count=1 errno=0".to_owned(),
        "LW_BASKET_IO kind=i2c_rdwr ret=1 count=1 errno=0".to_owned(),
        "LW_BASKET_IO kind=i2c_rdwr ret=2 count=2 errno=0".to_owned(),
        "LW_BASKET_DEVID value=229".to_owned(),
        "LW_BASKET_IO kind=i2c_rdwr ret=2 count=2 errno=0".to_owned(),
        format!("LW_BASKET_AXES raw={},{},{}", axes[0], axes[1], axes[2]),
    ];
    let mut tx = Vec::new();
    let mut rx = Vec::new();
    let mut peer = p.peer_before.clone();
    for exchange in phases["uart_exchanges"]
        .as_array()
        .ok_or("missing trusted exchanges")?
    {
        let command = exchange["command"].as_str().ok_or("missing command")?;
        let reply = exchange["reply"].as_str().ok_or("missing reply")?;
        tx.extend_from_slice(format!("{command}\r\n").as_bytes());
        rx.extend_from_slice(reply.as_bytes());
        peer.push(format!("{command} -> {}", reply.trim_end()));
        lines.push(format!(
            "LW_BASKET_UART command={command} written={} received={} errno=0 reply={}",
            command.len() + 2,
            reply.len(),
            reply.replace('\r', "\\r").replace('\n', "\\n")
        ));
    }
    require(
        actual_lines == lines,
        "actual Linux I2C_RDWR count/DEVID/axes or termios exchange mismatch",
    )?;
    require(
        uart(BusDir::Tx) == tx && uart(BusDir::Rx) == rx,
        "actual UART4 attempted TX/enqueued RX mismatch",
    )?;
    require(
        p.peer_after == peer,
        "same canonical behavioral peer accepted command log mismatch",
    )?;
    Ok(())
}

#[derive(Default)]
pub(super) struct MarkerTracker {
    next: usize,
    active: Option<usize>,
    helper: bool,
    exited: bool,
    accepted: bool,
}
#[derive(Debug, PartialEq)]
pub(super) enum Marker {
    Ready(usize),
    Done(usize),
    Accepted,
}
impl MarkerTracker {
    pub(super) fn consume(&mut self, line: &str) -> Result<Option<Marker>, String> {
        for (prefix, ready) in [
            ("LW_BASKET_PHASE_READY ", true),
            ("LW_BASKET_PHASE_DONE ", false),
        ] {
            if line.starts_with(prefix) {
                if self.next >= 4
                    || line != format!("{prefix}seq={} id={}", self.next + 1, PHASE_IDS[self.next])
                {
                    return Err(format!("unexpected exact phase marker: {line}"));
                }
                if ready {
                    if self.active.is_some() {
                        return Err("duplicate READY".into());
                    }
                    self.active = Some(self.next);
                    return Ok(Some(Marker::Ready(self.next)));
                }
                if self.active != Some(self.next) {
                    return Err("DONE without matching READY".into());
                }
                let index = self.next;
                self.next += 1;
                self.active = None;
                return Ok(Some(Marker::Done(index)));
            }
        }
        if line == "LW_BASKET_HELPER_ACCEPTED" && self.next == 4 && !self.helper {
            self.helper = true;
            return Ok(None);
        }
        if line == "LW_BASKET_HELPER_EXIT=0" && self.helper && !self.exited {
            self.exited = true;
            return Ok(None);
        }
        if line == "LW_BASKET_GUEST_ACCEPTED" && self.exited && !self.accepted {
            self.accepted = true;
            return Ok(Some(Marker::Accepted));
        }
        if line.starts_with("LW_BASKET_PHASE")
            || line.starts_with("LW_BASKET_HELPER")
            || line.starts_with("LW_BASKET_GUEST")
        {
            return Err(format!("out-of-order or duplicate basket marker: {line}"));
        }
        Ok(None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use labwired_core::bus::bus_trace::{BusDir, BusPayload};
    fn off() -> PhaseProof {
        PhaseProof {
            action_cycle: 100,
            from_seq: 10,
            through_seq: 14,
            first_retained_seq: 1,
            evicted: 0,
            events: b"AT\r\n"
                .iter()
                .enumerate()
                .map(|(n, b)| BusTraceEvent {
                    seq: 11 + n as u64,
                    cycle: 2_400_100 + n as u64,
                    bus: "uart4".into(),
                    payload: BusPayload::Uart {
                        direction: BusDir::Tx,
                        byte: *b,
                    },
                })
                .collect(),
            peer_before: vec!["accepted prior frame".into()],
            peer_after: vec!["accepted prior frame".into()],
            sensor: serde_json::json!({"powered":false,"supply_generation":2}),
            radio: serde_json::json!({"powered":false,"supply_generation":2,"command_bytes":0,"reply_bytes":0,"mock_connected":false}),
            raw5: 5.,
            raw3: 3.3,
            effective3: 0.,
            host_held: false,
        }
    }
    const OFF:&str="LW_BASKET_IO kind=i2c_rdwr ret=-6 count=2 errno=6\nLW_BASKET_UART command=AT written=4 received=0 errno=0 reply=\n";
    #[test]
    fn exact_unique_ordered_phase_markers_do_not_treat_text_as_a_cpu_stop() {
        let mut tracker = MarkerTracker::default();
        for (i, id) in [
            "stimulus_a",
            "stimulus_b",
            "external_off",
            "external_restored",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(
                tracker
                    .consume(&format!("LW_BASKET_PHASE_READY seq={} id={id}", i + 1))
                    .unwrap(),
                Some(Marker::Ready(i))
            );
            assert!(tracker
                .consume(&format!("LW_BASKET_PHASE_READY seq={} id={id}", i + 1))
                .is_err());
            assert_eq!(
                tracker
                    .consume(&format!("LW_BASKET_PHASE_DONE seq={} id={id}", i + 1))
                    .unwrap(),
                Some(Marker::Done(i))
            );
        }
        assert_eq!(tracker.consume("LW_BASKET_HELPER_ACCEPTED").unwrap(), None);
        assert_eq!(tracker.consume("LW_BASKET_HELPER_EXIT=0").unwrap(), None);
        assert_eq!(
            tracker.consume("LW_BASKET_GUEST_ACCEPTED").unwrap(),
            Some(Marker::Accepted)
        );
        assert!(tracker.consume("LW_BASKET_GUEST_ACCEPTED").is_err());
        for line in [
            "LW_BASKET_GUEST_ACCEPTED",
            "LW_BASKET_PHASE_DONE seq=1 id=stimulus_a",
            "LW_BASKET_PHASE_READY seq=2 id=stimulus_b",
            "LW_BASKET_PHASE_READY seq=1 id=stimulus_a trailing",
        ] {
            assert!(MarkerTracker::default().consume(line).is_err(), "{line}");
        }
    }
    #[test]
    fn powered_guest_text_alone_cannot_replace_actual_shared_trace_registers_and_component_inputs()
    {
        use labwired_core::bus::I2cSym;
        let mut p = off();
        p.events.clear();
        p.effective3 = 3.3;
        p.sensor = serde_json::json!({"powered":true,"supply_generation":1,"inputs":{"x":1.,"y":-0.5,"z":0.25},"registers":{"DEVID":229,"POWER_CTL":8,"DATA_FORMAT":8,"DATAX0":256,"DATAY0":-128,"DATAZ0":64}});
        p.radio = serde_json::json!({"powered":true,"supply_generation":1,"mock_connected":false,"command_bytes":0,"reply_bytes":0});
        let a = I2cSym::AddrWrite;
        let r = I2cSym::AddrRead;
        let d = I2cSym::Data;
        for (kind, byte) in [
            (a, 166),
            (d, 45),
            (d, 8),
            (a, 166),
            (d, 49),
            (d, 8),
            (a, 166),
            (d, 0),
            (r, 167),
            (d, 229),
            (a, 166),
            (d, 50),
            (r, 167),
            (d, 0),
            (d, 1),
            (d, 128),
            (d, 255),
            (d, 64),
            (d, 0),
        ] {
            let seq = p.from_seq + p.events.len() as u64 + 1;
            p.events.push(BusTraceEvent {
                seq,
                cycle: 2_400_100 + seq,
                bus: "i2c3".into(),
                payload: BusPayload::I2c {
                    kind,
                    byte,
                    ack: true,
                },
            });
        }
        let mut console="LW_BASKET_IO kind=i2c_rdwr ret=1 count=1 errno=0\nLW_BASKET_IO kind=i2c_rdwr ret=1 count=1 errno=0\nLW_BASKET_IO kind=i2c_rdwr ret=2 count=2 errno=0\nLW_BASKET_DEVID value=229\nLW_BASKET_IO kind=i2c_rdwr ret=2 count=2 errno=0\nLW_BASKET_AXES raw=256,-128,64\n".to_string();
        let phases: Value = serde_json::from_str(include_str!(
            "../../../../examples/luckfox-pico-mini-a-linux/basket/phases.json"
        ))
        .unwrap();
        for exchange in phases["uart_exchanges"].as_array().unwrap() {
            let command = exchange["command"].as_str().unwrap();
            let reply = exchange["reply"].as_str().unwrap();
            console.push_str(&format!(
                "LW_BASKET_UART command={command} written={} received={} errno=0 reply={}\n",
                command.len() + 2,
                reply.len(),
                reply.replace('\r', "\\r").replace('\n', "\\n")
            ));
            p.peer_after
                .push(format!("{command} -> {}", reply.trim_end()));
            for (direction, bytes) in [
                (BusDir::Tx, format!("{command}\r\n").into_bytes()),
                (BusDir::Rx, reply.as_bytes().to_vec()),
            ] {
                for byte in bytes {
                    let seq = p.from_seq + p.events.len() as u64 + 1;
                    p.events.push(BusTraceEvent {
                        seq,
                        cycle: 2_400_100 + seq,
                        bus: "uart4".into(),
                        payload: BusPayload::Uart { direction, byte },
                    });
                }
            }
        }
        p.through_seq = p.events.last().unwrap().seq;
        assert!(validate_phase(0, &console, &p).is_ok());
        for mutation in 0..6 {
            let mut bad = p.clone();
            match mutation {
                0 => bad.events.clear(),
                1 => bad.sensor["registers"]["POWER_CTL"] = 0.into(),
                2 => bad.sensor["inputs"]["x"] = 0.into(),
                3 => bad.peer_after = bad.peer_before.clone(),
                4 => bad.events[0].cycle = bad.action_cycle,
                _ => bad.radio["reply_bytes"] = 1.into(),
            };
            assert!(
                validate_phase(0, &console, &bad).is_err(),
                "independent powered predicate {mutation}"
            );
        }
        assert!(validate_phase(0, &console.replace("ret=2 count=2", "ret=1 count=2"), &p).is_err());
    }
    #[test]
    fn real_off_phase_requires_complete_trace_coverage_and_unchanged_peer_not_absent_tx_attempt() {
        assert!(validate_phase(2, OFF, &off()).is_ok());
        for mutation in 0..7 {
            let mut p = off();
            match mutation {
                0 => p.first_retained_seq = 12,
                1 => p.peer_after.push("accepted AT".into()),
                2 => {
                    p.events[0].payload = BusPayload::Uart {
                        direction: BusDir::Rx,
                        byte: b'O',
                    };
                }
                3 => p.effective3 = 3.3,
                4 => p.host_held = true,
                5 => {
                    p.events.remove(0);
                }
                _ => p.raw3 = 0.,
            };
            assert!(
                validate_phase(2, OFF, &p).is_err(),
                "independent predicate {mutation}"
            );
        }
        assert!(validate_phase(2, &OFF.replace("ret=-6", "ret=2"), &off()).is_err());
        assert!(validate_phase(2, &OFF.replace("received=0", "received=2"), &off()).is_err());
    }
}
