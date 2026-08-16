// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Ratchet over the undecoded MMIO register accesses the shipped corpus performs.
//!
//! # What this enforces
//!
//! `docs/coverage/undecoded-registers.json` is the committed allow-list: the 5
//! distinct `(peripheral, offset, kind)` triples the silent-path census found
//! across 68 runs. This file holds that list to three rules:
//!
//! 1. **Every entry still reproduces.** Each one is replayed against the real
//!    model, and the census must record it. The day someone implements the
//!    register — or removes the guard that was rejecting the write — the replay
//!    stops being recorded and this test goes RED, forcing the entry out of the
//!    list. That is the shrink direction, and it is the whole point: a fixed
//!    register that stays on the list turns the list into an excuse.
//! 2. **The list cannot grow silently.** `max_entries` is committed alongside
//!    it and must be lowered whenever an entry is removed.
//! 3. **Every entry is classified and justified.** An entry with no reason is
//!    an entry nobody has looked at.
//!
//! # What this does NOT do
//!
//! It does not *discover* new undecoded accesses. Discovery needs the full
//! census campaign — build the CLI with `--features silent-census`, run the
//! corpus with `LABWIRED_CENSUS_OUT` set, aggregate. That is documented in
//! `docs/coverage/silent-path-census.md` and costs far more than a test lane.
//! What this catches is the *other* direction: the list going stale while the
//! models move underneath it.
//!
//! Stating that plainly matters. A gate whose limits are not written down gets
//! read as covering more than it does — which is how a green tick starts
//! meaning "nobody checked".
//!
//! # Why a replay and not a corpus run
//!
//! Reproducing an entry needs the peripheral, the offset and the direction —
//! all three are in the allow-list, and every one of these models is
//! constructible in isolation. Replaying costs microseconds; running the five
//! labs that hit these offsets costs ~9.5M simulated steps and needs their
//! firmware ELFs. The replay tests the same claim: *is this offset still
//! undecoded on this model?*
//!
//! Runs in the nightly `silent-census` lane, through
//! `scripts/ci/cargo-test-nonvacuous.sh`, so an empty binary fails instead of
//! reporting `0 passed`.

#![cfg(feature = "silent-census")]

use labwired_core::Peripheral;

/// The census is process-global, and every test here asserts absolute totals
/// after a `reset()`. `cargo test` runs a binary's tests concurrently, so
/// without this they read each other's tallies.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialized() -> std::sync::MutexGuard<'static, ()> {
    let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    labwired_core::census::reset();
    g
}

const ALLOW_LIST: &str = include_str!("../../../docs/coverage/undecoded-registers.json");

const CLASSIFICATIONS: &[&str] = &["guard_reject", "not_a_register", "unmodelled_register"];

fn allow_list() -> serde_json::Value {
    serde_json::from_str(ALLOW_LIST)
        .expect("docs/coverage/undecoded-registers.json is not valid JSON")
}

fn entries(v: &serde_json::Value) -> &Vec<serde_json::Value> {
    v["entries"].as_array().expect("`entries` must be an array")
}

/// Replay one allow-list entry against the real model it names.
///
/// Every peripheral is built fresh and left in its reset state, which is what
/// makes the `guard_reject` entries reproduce: `Iwdg` boots with the write
/// unlock clear and `Fdcan` with `CCCR.TEST` clear, so those writes fall to the
/// catch-all exactly as they do on the labs that recorded them.
fn replay(peripheral: &str, offset: u64, kind: &str) {
    assert_eq!(
        kind, "write",
        "only write replays are implemented; {peripheral}@{offset:#06x} is a {kind}"
    );
    let mut model: Box<dyn Peripheral> = match peripheral {
        "iwdg:Iwdg" => Box::new(labwired_core::peripherals::iwdg::Iwdg::new()),
        "fdcan:Fdcan" => Box::new(labwired_core::peripherals::fdcan::Fdcan::new()),
        "nrf52.twim:Nrf52Twim" => {
            Box::new(labwired_core::peripherals::nrf52::twim::Nrf52Twim::new())
        }
        "nrf54l.twim:Nrf54lTwim" => {
            Box::new(labwired_core::peripherals::nrf54l::twim::Nrf54lTwim::new())
        }
        other => panic!(
            "the allow-list names peripheral `{other}`, which this test cannot construct. \
             Add it to `replay` — an entry nothing can replay is an entry nothing enforces."
        ),
    };
    model
        .write_u32(offset, 0xDEAD_BEEF)
        .unwrap_or_else(|e| panic!("{peripheral}@{offset:#06x} write failed: {e:?}"));
}

fn parse_offset(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16)
        .unwrap_or_else(|_| panic!("offset `{s}` is not hex"))
}

/// The load-bearing test: each committed entry must STILL be undecoded.
#[test]
fn every_allow_listed_entry_is_still_undecoded() {
    let _guard = serialized();
    let list = allow_list();

    for e in entries(&list) {
        labwired_core::census::reset();
        let periph = e["peripheral"].as_str().expect("peripheral");
        let offset_str = e["offset"].as_str().expect("offset");
        let kind = e["kind"].as_str().expect("kind");

        replay(periph, parse_offset(offset_str), kind);

        let j = labwired_core::census::to_json();
        let hit = j["undecoded_register_access"]["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .any(|r| {
                r["peripheral"] == *periph && r["offset"] == *offset_str && r["kind"] == *kind
            });

        assert!(
            hit,
            "{periph}@{offset_str} ({kind}) is on the allow-list in \
             docs/coverage/undecoded-registers.json but the model no longer falls through on it. \
             If the register was implemented, or the guard that was rejecting the write is gone, \
             DELETE this entry and lower `max_entries`. The list only shrinks."
        );
    }
}

/// The probe must be able to tell decoded from undecoded, or the test above
/// passes for the wrong reason.
#[test]
fn a_decoded_offset_is_not_recorded() {
    let _guard = serialized();

    // IWDG KR at 0x00 is decoded unconditionally (it is the unlock register
    // itself, so it carries no guard) — the one offset on this model that
    // cannot fall through.
    let mut iwdg = labwired_core::peripherals::iwdg::Iwdg::new();
    iwdg.write_u32(0x00, 0x5555).unwrap();
    assert_eq!(
        labwired_core::census::to_json()["undecoded_register_access"]["total"],
        0,
        "a decoded offset was recorded as undecoded — the counter is firing on everything, \
         which would make the allow-list test vacuously green"
    );

    // And the same model DOES record a genuinely unmapped offset, so the zero
    // above is a real negative and not a dead counter.
    //
    // Asserted per-key, not on `total`: a 32-bit access is serviced as four
    // byte accesses, and on this model `write_u32` is a read-modify-write, so
    // one call lands 4 reads AND 4 writes on the same key. That multiplier is
    // why `silent-path-census.md` reads raw counts with a 4x divide — it is a
    // property of the bus width, not of how undecoded the offset is, so
    // pinning an exact total here would encode an unrelated implementation
    // detail and break the day a model stops round-tripping.
    iwdg.write_u32(0xF0, 0x1).unwrap();
    let j = labwired_core::census::to_json();
    let hit = j["undecoded_register_access"]["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|r| r["offset"] == "0x00f0" && r["kind"] == "write")
        .and_then(|r| r["count"].as_u64())
        .unwrap_or(0);
    assert!(
        hit > 0,
        "the counter never fired on a known-unmapped offset; census entries were {}",
        j["undecoded_register_access"]["entries"]
    );
}

/// Structural rules: the list cannot grow by editing, and cannot rot into a
/// bare list of offsets nobody has reasoned about.
#[test]
fn the_allow_list_only_shrinks_and_every_entry_is_justified() {
    let list = allow_list();
    let es = entries(&list);
    let max = list["max_entries"].as_u64().expect("max_entries") as usize;

    assert!(
        !es.is_empty(),
        "the allow-list is empty. If the corpus really performs zero undecoded accesses that is \
         excellent news — but delete this test with it, because an empty list makes \
         `every_allow_listed_entry_is_still_undecoded` iterate nothing and report green."
    );
    assert!(
        es.len() <= max,
        "{} entries against max_entries {}. The list grew. A new undecoded access is a finding, \
         not a line to add: fix the model, or raise max_entries in the same commit that explains why.",
        es.len(),
        max
    );
    assert_eq!(
        es.len(),
        max,
        "max_entries is {} but the list holds {}. When an entry is removed, lower max_entries in \
         the same commit — otherwise the slack silently re-admits it later.",
        max,
        es.len()
    );

    for e in es {
        let periph = e["peripheral"].as_str().expect("peripheral");
        let off = e["offset"].as_str().expect("offset");
        let class = e["classification"].as_str().unwrap_or("");
        assert!(
            CLASSIFICATIONS.contains(&class),
            "{periph}@{off} has classification `{class}`, which is not one of {CLASSIFICATIONS:?}"
        );
        let reason = e["reason"].as_str().unwrap_or("");
        assert!(
            reason.len() > 80,
            "{periph}@{off} has no real reason ({} chars). Say which register the offset is, on \
             whose authority, and what would have to change to remove the entry.",
            reason.len()
        );
        assert!(
            e["seen_in"].as_str().is_some_and(|s| s.contains(".yaml")),
            "{periph}@{off} must name the lab that recorded it, so the claim can be re-measured"
        );
    }
}
