//! **Migration parity for the three ported UART parts.**
//!
//! `components/{hc05,sim800l,neo6m}.rs` are deleted. The goldens below were
//! captured FROM THOSE MODELS, by running them, on the commit that removes
//! them — not transcribed from reading their source. Every byte here is what
//! the hand-written model actually put on the wire.
//!
//! The three parts share one harness because they shared one implementation:
//! the same line buffer, the same 128-byte cap, the same `poll` that pops a
//! byte. Only the command table differed, and a table is data.
//!
//! Where the port DIFFERS from the model it replaces, the difference gets its
//! own test with its own name, rather than being absorbed into a golden.

use labwired_core::peripherals::components::declarative_uart::{
    DeclarativeUartDevice, DeclarativeUartKit,
};
use labwired_core::peripherals::device::UartStreamDevice;
use labwired_core::sim_input::SimInput;

fn device(device_type: &str) -> DeclarativeUartDevice {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .unwrap_or_else(|| panic!("{device_type} descriptor is embedded"));
    DeclarativeUartKit::from_yaml(yaml)
        .unwrap_or_else(|e| panic!("{device_type}.yaml is a valid uart_device: {e:#}"))
        .device(device_type)
        .unwrap_or_else(|e| panic!("{device_type} builds: {e:#}"))
}

/// Write a line and read back everything the part answers, exactly as the
/// hand-written models' own tests did.
fn ask(dev: &mut DeclarativeUartDevice, line: &str) -> String {
    for b in line.bytes() {
        dev.on_tx_byte(b);
    }
    let mut out = String::new();
    while let Some(b) = dev.poll(0) {
        out.push(b as char);
    }
    out
}

/// Credit one sentence period and read out whatever the part then says.
fn tick(dev: &mut DeclarativeUartDevice, us: u32) -> String {
    let mut out = String::new();
    if let Some(b) = dev.poll(us) {
        out.push(b as char);
    }
    while let Some(b) = dev.poll(0) {
        out.push(b as char);
    }
    out
}

// ── HC-05 ───────────────────────────────────────────────────────────────────

/// Every probe the hand-written `Hc05` was driven with, and the exact bytes it
/// answered. **Byte-identical**: there is no deliberate difference here.
const HC05_GOLDEN: &[(&str, &str)] = &[
    ("AT\r\n", "OK\r\n"),
    ("AT\r", "OK\r\n"),
    ("AT\n", "OK\r\n"),
    ("at\r\n", "OK\r\n"),
    ("AT+VERSION\r\n", "+VERSION:labwired-hc05-sim\r\nOK\r\n"),
    ("AT+VERSION?\r\n", "+VERSION:labwired-hc05-sim\r\nOK\r\n"),
    ("AT+NAME?\r\n", "+NAME:HC-05\r\nOK\r\n"),
    // A NAME *write* is answered OK and not stored — see the descriptor header.
    ("AT+NAME=Robot\r\n", "OK\r\n"),
    ("AT+CGMI\r\n", "OK\r\n"),
    ("AT+CGMM\r\n", "OK\r\n"),
    // `ATI` is not an `AT`/`AT+`/`AT ` line, so the HC-05 rejects it. That was
    // right in the old model and is kept.
    ("ATI\r\n", "ERROR\r\n"),
    ("AT+GMR\r\n", "OK\r\n"),
    ("AT+CSQ\r\n", "OK\r\n"),
    ("AT+CSQ=?\r\n", "OK\r\n"),
    ("AT+CREG?\r\n", "OK\r\n"),
    ("AT+CREG=2\r\n", "OK\r\n"),
    ("AT+RESET\r\n", "OK\r\n"),
    ("AT UART\r\n", "OK\r\n"),
    ("hello\r\n", "ERROR\r\n"),
    // A frame that trims to nothing answers nothing. Without that rule the
    // `\n` of every `AT\r\n` would answer a second time.
    ("\r\n", ""),
    ("   \r\n", ""),
];

#[test]
fn hc05_transcript_is_byte_identical_to_the_deleted_model() {
    let mut dev = device("hc-05");
    for (line, expected) in HC05_GOLDEN {
        assert_eq!(
            &ask(&mut dev, line),
            expected,
            "HC-05 answered {line:?} differently from the model it replaces"
        );
    }
}

// ── SIM800L ─────────────────────────────────────────────────────────────────

/// The hand-written `Sim800l`'s transcript. One entry is DELIBERATELY absent —
/// `ATI`, which the old model answered `ERROR`; see
/// [`sim800l_ati_answered_error_and_now_answers_the_banner`].
const SIM800L_GOLDEN: &[(&str, &str)] = &[
    ("AT\r\n", "OK\r\n"),
    ("AT\r", "OK\r\n"),
    ("AT\n", "OK\r\n"),
    ("at\r\n", "OK\r\n"),
    ("AT+VERSION\r\n", "OK\r\n"),
    ("AT+VERSION?\r\n", "OK\r\n"),
    ("AT+NAME?\r\n", "OK\r\n"),
    ("AT+NAME=Robot\r\n", "OK\r\n"),
    ("AT+CGMI\r\n", "SIMCOM_Ltd\r\nOK\r\n"),
    ("AT+CGMM\r\n", "SIMCOM_SIM800L\r\nOK\r\n"),
    ("AT+GMR\r\n", "SIM800L R14.18 LabWired\r\nOK\r\n"),
    ("AT+CSQ\r\n", "+CSQ: 20,0\r\nOK\r\n"),
    ("AT+CSQ=?\r\n", "+CSQ: 20,0\r\nOK\r\n"),
    ("AT+CREG?\r\n", "+CREG: 0,1\r\nOK\r\n"),
    ("AT+CREG=2\r\n", "+CREG: 0,1\r\nOK\r\n"),
    ("AT+RESET\r\n", "OK\r\n"),
    ("AT UART\r\n", "OK\r\n"),
    ("hello\r\n", "ERROR\r\n"),
    ("\r\n", ""),
    ("   \r\n", ""),
];

#[test]
fn sim800l_transcript_is_byte_identical_to_the_deleted_model() {
    let mut dev = device("sim800l");
    for (line, expected) in SIM800L_GOLDEN {
        assert_eq!(
            &ask(&mut dev, line),
            expected,
            "SIM800L answered {line:?} differently from the model it replaces"
        );
    }
}

/// ⚠️ **Deliberate difference — a fixed bug.**
///
/// The hand-written model gated every command behind
/// `upper == "AT" || starts_with("AT+") || starts_with("AT ")` and only THEN
/// tested `upper == "ATI"`. `ATI` passes none of the three, so that arm was
/// unreachable and the most common identification command there is answered
/// `ERROR`. `AT+GMR`, its documented alias, reached the arm and worked — which
/// is why nothing noticed.
///
/// The old answer is held here as a golden so the change is recorded rather
/// than absorbed.
#[test]
fn sim800l_ati_answered_error_and_now_answers_the_banner() {
    const WHAT_THE_DELETED_MODEL_ANSWERED: &str = "ERROR\r\n";
    let mut dev = device("sim800l");
    let now = ask(&mut dev, "ATI\r\n");
    assert_ne!(now, WHAT_THE_DELETED_MODEL_ANSWERED);
    assert_eq!(
        now, "SIM800L R14.18 LabWired\r\nOK\r\n",
        "ATI answers the same banner AT+GMR always did"
    );
}

/// ⚠️ Every network answer is a CONSTANT, named as one. A part that faked a
/// signal model would have this test asserting that the number MOVES; it does
/// not, and the descriptor says so.
#[test]
fn sim800l_signal_and_registration_are_constants_that_never_move() {
    let mut dev = device("sim800l");
    for _ in 0..5 {
        assert_eq!(ask(&mut dev, "AT+CSQ\r\n"), "+CSQ: 20,0\r\nOK\r\n");
        assert_eq!(ask(&mut dev, "AT+CREG?\r\n"), "+CREG: 0,1\r\nOK\r\n");
    }
    // Setting the URC mode does not change what CREG reports either: there is
    // no registration state machine behind it.
    assert_eq!(ask(&mut dev, "AT+CREG=2\r\n"), "+CREG: 0,1\r\nOK\r\n");
    assert_eq!(ask(&mut dev, "AT+CREG?\r\n"), "+CREG: 0,1\r\nOK\r\n");
}

/// A GPRS/socket command answers a bare `OK` with no value line — the honest
/// "not implemented", and what the model it replaces did.
#[test]
fn sim800l_answers_an_unimplemented_command_with_a_bare_ok() {
    let mut dev = device("sim800l");
    for cmd in [
        "AT+CGATT?\r\n",
        "AT+CIPSTART=\"TCP\",\"example.com\",80\r\n",
        "AT+CMGS=\"+10000000000\"\r\n",
    ] {
        assert_eq!(ask(&mut dev, cmd), "OK\r\n", "{cmd:?}");
    }
}

// ── NEO-6M ──────────────────────────────────────────────────────────────────

/// The hand-written `Neo6mGps` at its defaults — San Francisco, active fix —
/// four sentences. **Byte-identical**, including the XOR checksums, which is
/// the whole test: the port builds `DDMM.mmmm` from integer arithmetic where
/// the model used `f64` and `format!("{:09.4}")`.
const NEO6M_SF: &[&str] = &[
    "$GPGGA,120000.00,3746.4940,N,12225.1640,W,1,08,1.0,10.0,M,0.0,M,,*7F\r\n",
    "$GPRMC,120000.00,A,3746.4940,N,12225.1640,W,0.0,0.0,150526,,,A*40\r\n",
    "$GPGGA,120000.00,3746.4940,N,12225.1640,W,1,08,1.0,10.0,M,0.0,M,,*7F\r\n",
    "$GPRMC,120000.00,A,3746.4940,N,12225.1640,W,0.0,0.0,150526,,,A*40\r\n",
];

#[test]
fn neo6m_default_sentences_are_byte_identical_to_the_deleted_model() {
    let mut dev = device("neo6m-gps");
    for (i, expected) in NEO6M_SF.iter().enumerate() {
        assert_eq!(&tick(&mut dev, 500_000), expected, "sentence {i}");
    }
}

/// London with the fix dropped: a WESTERN longitude below one degree (so the
/// `{:010.4}` zero padding is what is being checked) and the `V` status letter.
#[test]
fn neo6m_a_sub_degree_west_position_without_a_fix_is_byte_identical() {
    let mut dev = device("neo6m-gps");
    dev.set_input("lat", 51.5074).unwrap();
    dev.set_input("lon", -0.1278).unwrap();
    dev.set_input("fix", 0.0).unwrap();
    assert_eq!(
        tick(&mut dev, 500_000),
        "$GPGGA,120000.00,5130.4440,N,00007.6680,W,0,08,1.0,10.0,M,0.0,M,,*78\r\n"
    );
    assert_eq!(
        tick(&mut dev, 500_000),
        "$GPRMC,120000.00,V,5130.4440,N,00007.6680,W,0.0,0.0,150526,,,A*51\r\n"
    );
}

/// Exactly zero is NORTH and EAST — `deg >= 0.0` in the deleted model, and
/// `input(lat) >= 0` in the descriptor. A sign convention that flipped at the
/// origin would be invisible everywhere else.
#[test]
fn neo6m_the_origin_reads_north_and_east() {
    let mut dev = device("neo6m-gps");
    dev.set_input("lat", 0.0).unwrap();
    dev.set_input("lon", 0.0).unwrap();
    assert_eq!(
        tick(&mut dev, 500_000),
        "$GPGGA,120000.00,0000.0000,N,00000.0000,E,1,08,1.0,10.0,M,0.0,M,,*67\r\n"
    );
    assert_eq!(
        tick(&mut dev, 500_000),
        "$GPRMC,120000.00,A,0000.0000,N,00000.0000,E,0.0,0.0,150526,,,A*58\r\n"
    );
}

/// The GGA quality digit and the RMC status letter are the SAME channel, and
/// the deleted model's threshold was `value >= 0.5`.
#[test]
fn neo6m_the_fix_channel_drives_both_sentences() {
    let mut dev = device("neo6m-gps");
    dev.set_input("fix", 0.4).unwrap();
    assert!(
        tick(&mut dev, 500_000).contains(",W,0,08,"),
        "GGA quality 0"
    );
    assert!(tick(&mut dev, 500_000).contains(",V,"), "RMC status V");
    dev.set_input("fix", 0.5).unwrap();
    assert!(
        tick(&mut dev, 500_000).contains(",W,1,08,"),
        "GGA quality 1"
    );
    assert!(tick(&mut dev, 500_000).contains(",A,"), "RMC status A");
}

/// ⚠️ **Deliberate difference — the clock no longer drifts.**
///
/// The deleted model accumulated `elapsed_us` ONLY while its output queue was
/// empty and reset the accumulator to zero after each sentence, so its period
/// was really 500 ms PLUS however long the previous sentence took to clock
/// out. Driven at one byte per millisecond — which is what the hosting UART
/// does — its sentences started at poll 500, 1069 and 1635: gaps of 569 ms and
/// 566 ms for a receiver that is supposed to be on a 500 ms grid.
///
/// The port credits time unconditionally and `TimerBank` reschedules from the
/// DEADLINE, so the sentences start at 500, 1000 and 1500.
#[test]
fn neo6m_sentences_land_on_a_true_500ms_grid() {
    /// Poll numbers at which the DELETED model started a sentence, at 1 ms per
    /// poll. Captured by running it.
    const DRIFTING: [usize; 3] = [500, 1069, 1635];

    let mut dev = device("neo6m-gps");
    let mut starts = Vec::new();
    for poll in 1..=2_000usize {
        if dev.poll(1000) == Some(b'$') {
            starts.push(poll);
            if starts.len() == 3 {
                break;
            }
        }
    }
    assert_ne!(starts, DRIFTING, "the drift is what this port removes");
    assert_eq!(
        starts,
        vec![500, 1000, 1500],
        "one sentence every 500 ms regardless of how long the last took to \
         clock out"
    );
}

/// The two entries alternate because both guards see the SAME `idx` — they are
/// evaluated before the `timer:` rule increments it. Getting that order wrong
/// emits both sentences every tick, which is the one way this descriptor can
/// silently double its output rate.
#[test]
fn neo6m_gga_and_rmc_alternate_one_per_tick() {
    let mut dev = device("neo6m-gps");
    let mut kinds = Vec::new();
    for _ in 0..6 {
        let s = tick(&mut dev, 500_000);
        assert_eq!(
            s.matches('$').count(),
            1,
            "exactly one sentence per tick: {s}"
        );
        kinds.push(if s.contains("GPGGA") { "GGA" } else { "RMC" });
    }
    assert_eq!(kinds, ["GGA", "RMC", "GGA", "RMC", "GGA", "RMC"]);
}

/// Seeded noise still moves the position and still replays: the same facility
/// the IMU kits use, sampled ONCE per sentence so a 70-byte drain cannot show
/// two different positions.
#[test]
fn neo6m_noise_moves_the_sentence_and_replays_bit_for_bit() {
    let build = |sigma: f64| {
        let yaml = labwired_config::embedded_device_yaml("neo6m-gps").unwrap();
        let mut d = DeclarativeUartKit::from_yaml(yaml)
            .unwrap()
            .device("gps")
            .unwrap();
        d.set_channel_noise_sigma("lat", sigma);
        d.set_channel_noise_sigma("lon", sigma);
        d
    };
    let mut a = build(0.01);
    let mut b = build(0.01);
    let mut quiet = build(0.0);
    let (sa, sb, sq) = (
        tick(&mut a, 500_000),
        tick(&mut b, 500_000),
        tick(&mut quiet, 500_000),
    );
    assert_eq!(sq, NEO6M_SF[0], "sigma 0 is byte-identical to no noise");
    assert_ne!(sa, sq, "noise never moved the sentence: {sa}");
    assert_eq!(sa, sb, "the same seed and id must replay bit-identically");
}

/// Two GPS modules on one board must not report the same noisy position: the
/// seed is keyed by component id, exactly as the deleted model's was.
#[test]
fn neo6m_two_modules_with_different_ids_diverge() {
    let build = |id: &str| {
        let yaml = labwired_config::embedded_device_yaml("neo6m-gps").unwrap();
        let mut d = DeclarativeUartKit::from_yaml(yaml)
            .unwrap()
            .device(id)
            .unwrap();
        d.set_channel_noise_sigma("lat", 0.01);
        d
    };
    let mut a = build("gps0");
    let mut b = build("gps1");
    assert_ne!(tick(&mut a, 500_000), tick(&mut b, 500_000));
}

/// The receiver ignores everything firmware transmits — it declares no
/// `responses:` at all, which is the truth for a NEO-6M running stock NMEA
/// output. A UBX command gets silence, not an invented ACK.
#[test]
fn neo6m_ignores_what_firmware_transmits() {
    let mut dev = device("neo6m-gps");
    assert_eq!(ask(&mut dev, "$PUBX,40,GLL,0,0,0,0*5C\r\n"), "");
}
