# iolinki-master Threat Model

**Aligned to:** IO-Link Security Design and Development Guideline, Order No. 10.512
(D1.0.0-01, October 2025) and IO-Link Secure Deployment Guideline, Order No. 10.502
(V1.0.0, June 2025), both published by the IO-Link Community. This document
paraphrases and cites those guidelines; it does not reproduce their text. Obtain
them from [io-link.com/downloads](https://io-link.com/downloads).

**Scope:** the `iolinki-master` stack as a software component integrated into an
IO-Link Master port controller. This is the *Master* side of the link: the peer on
the wire is a potentially rogue or faulty **Device**, and the primary job of the
stack is to ensure that no byte sequence a Device puts on the C/Q line can corrupt
master state or the integrating application's memory. The protocol in scope is the
wired point-to-point SDCI interface only — no networking, no TCP/IP, no wireless,
no fieldbus uplink (that uplink is the master product's domain, not this stack's).

Every stack claim below carries a code anchor. A claim without an anchor belongs in
[§5 Gaps](#5-gaps-and-integrator-duties), not here. Because `iolinki-master` is
simulation-validated and **not yet validated on real IO-Link silicon**, this model
is explicit about which guarantees are protocol-logic guarantees (locally tested)
and which depend on unverified PHY/timing behavior.

## 1. System model and trust boundaries

```
  Fieldbus / application host  (trusted: the master product's own firmware)
    │ caller API (iolink_master_* calls, caller-owned storage)
  Public API                include/iolinki_master/master.h
    │
  Protocol core             src/master_port.c  (startup, cyclic PD, RX/retry)
    ├─ Direct Parameters     src/master_parameters.c  (page 1 parse, identity)
    ├─ ISDU / services       src/master_isdu.c  (segmentation, DS, events, blocks)
    ├─ SIO DI/DQ             src/master_sio.c
    └─ Multi-port controller src/master_controller.c
    │ encoded frames / decoded bytes
  Shared frame + CRC        ../iolinki: src/frame.c, src/crc.c  (build-time reuse)
    │ bytes
  PHY adapter               iolink_phy_api_t + config adapter hooks (board code)
    │ C/Q wire  ═══════════════════════════════════ (untrusted input boundary)
  IO-Link Device            (rogue / faulty / attacker-controlled)
```

Trust boundaries:

- **The wire and the Device are untrusted.** Every byte arriving through
  `recv_byte` / `iolink_master_on_rx` — wake response, Direct Parameter Page 1,
  cyclic PD, OD/ISDU responses, event details, Data Storage records — is
  Device-controlled and, in the threat scenarios of 10.512 clause 6, attacker
  controlled. A rogue Device is the master's primary adversary.
- **The PHY adapter is a boundary, not a defender.** Board code owns transceiver
  registers, half-duplex direction, and the physical wake pulse/timing. The core
  treats adapter returns as data: a negative `send`/`recv_byte`/checked-hook result
  drives error handling rather than being trusted.
- **The caller/application is trusted.** The stack runs in the master firmware's
  trust domain and does not defend against its own host. Caller-owned opaque
  storage (`iolink_master_port_t`, `iolink_master_controller_t`) means the host,
  not the stack, controls all allocation.
- **Build/supply chain** is outside the runtime model and covered by the SBOM (see
  `SECURITY.md` / [`CRA.md`](CRA.md)): zero third-party runtime dependencies — the
  only external source reuse is the sibling `iolinki` `frame.c`/`crc.c` at build
  time — so the supply-chain surface is these two repositories plus the integrator's
  toolchain.

## 2. Assets

| Asset | Where it lives |
|---|---|
| Integrity of decoded process data returned to the application | `src/master_port.c`, `iolink_master_get_pd_in` |
| Master port state machine (inactive→startup→preoperate→operate→error) | `src/master_port.c`, `iolink_master_get_state` |
| Device parameterization images (ISDU, Data Storage, block params) | `src/master_isdu.c` |
| Device identity / configuration match | `src/master_parameters.c` |
| Availability of the master port function | whole stack, fixed-resource design |

## 3. STRIDE analysis

10.512 clause 6 (Table 1) identifies spoofing of either peer, tampering/replay on
the wire, information disclosure on the wire, and denial of service as the relevant
threats, with **physical protection of cable and device as the guideline's
countermeasure at SL-C 1**. The SDCI protocol carries no cryptographic
authentication, integrity, or confidentiality (10.512 §7.4.2, §7.5.2). The stack
therefore cannot — and does not claim to — defend against a physically present
attacker; what it guarantees is that malformed or hostile Device traffic is
*rejected safely* rather than corrupting the master.

### S — Spoofing (a rogue Device impersonating the expected one)

- Protocol reality (10.512 §6): SDCI has no cryptographic peer authentication;
  physical protection is the countermeasure. Integrator duty.
- Stack guarantees — identity is checked at the *inspection* level the port
  configuration model defines, not cryptographically:
  - Direct Parameter Page 1 is parsed into a typed device-info record and the
    connected Device's VendorID/DeviceID are compared against the configured
    expected values whenever `inspection_level != NO_CHECK`
    (`src/master_parameters.c`, `iolink_master_validate_config_against_device_info`;
    grep anchors `vendor_id`/`device_id` compare at the identity check). A mismatch
    is rejected with `PARAM_ERR_VENDOR_ID` / `PARAM_ERR_DEVICE_ID` before the port
    is allowed into OPERATE with that Device.
  - **Honest limit:** `IDENTICAL` inspection is meant to additionally bind the
    Device SerialNumber (ISDU index 0x0015). That leg is **not yet wired**; today
    `IDENTICAL` enforces the same VendorID/DeviceID check as `TYPE_COMP`
    (documented at the `iolink_master_inspection_level_t` enum in `master.h`, and in
    [`IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md) "Device identity"). A
    Device that clones a VendorID/DeviceID pair is not distinguished from the
    genuine unit. This is an identity gap, not a memory-safety gap.

### T — Tampering (modified, replayed, or forged Device frames)

- Protocol reality: the per-frame CRC6/checksum detects *accidental* corruption
  only; intentional modification with a valid checksum is undetectable at the
  protocol level (10.512 §7.4.2). Integrator duty: physical protection.
- Stack guarantees (CR 3.1, CR 3.5 — 10.512 §7.4.2, §7.4.5), all locally tested:
  - Every received response is checksum-verified before use: Type-0 replies via
    `iolink_checksum_ck` and multi-octet OPERATE frames via `iolink_crc6` /
    `resp.checksum_ok` (`src/master_port.c`, RX path around the `checksum_errors`
    counter). A failed check increments `checksum_errors`, triggers bounded retry,
    and otherwise returns `IOLINK_MASTER_ERR_CHECKSUM` — it never forwards
    corrupted PD to the caller.
  - Frame decode is length/bounds-checked before parsing; NULL and size violations
    return `INVALID_ARG` / `ERR_FRAME` (`iolink_master_on_rx`, `iolink_master_poll_rx`).
  - ISDU responses accumulate into a fixed buffer with an explicit ceiling: the
    response length is clamped at `IOLINK_ISDU_BUFFER_SIZE` before each byte is
    stored (`src/master_isdu.c`, `iolink_master_isdu_on_od`), and the read-out path
    refuses to overflow the caller buffer — when `*len < result_len` it reports the
    required size and returns `ISDU_ERR_BUFFER_TOO_SMALL` instead of copying
    (`iolink_master_isdu_finish_read`). ISDU writes bound the request against
    `IOLINK_ISDU_BUFFER_SIZE - 5` before staging (`iolink_master_write_isdu`).
  - Event-detail decode is caller-bounded: `iolink_master_read_event_details` takes
    `max_events` and writes no more than that, returning `BUFFER_TOO_SMALL` rather
    than overrunning the caller's `events[]` array.
  - Data Storage restore verifies by readback: `iolink_master_restore_data_storage`
    and `iolink_master_verify_data_storage` compare the written image against a
    read-back copy and return `VERIFY_FAILED` on mismatch, so a Device that
    silently mangles a stored record is detected rather than trusted.

### R — Repudiation

- Assessed not relevant at SL-C 1 (10.512 §7.3.9): machine-to-machine, no human
  users, no accounts. No stack claims. Diagnostics counters
  (`iolink_master_get_diagnostics`) are operational, not an audit log.

### I — Information disclosure (wire eavesdropping)

- Protocol reality: no encryption exists (10.512 §7.5.2); confidentiality in
  transit is achieved by restricting physical access. Integrator duty, to be stated
  in the master product's user documentation.
- Stack guarantee (CR 3.7 — 10.512 §7.4.7): the stack originates no data of its own
  onto the wire beyond protocol-required master frames and the services the caller
  invokes. Error paths return the named result codes in `master.h`
  (`IOLINK_MASTER_ERR_*`, `IOLINK_MASTER_ISDU_ERR_*`); no internal pointers or
  private `src/master_internal.h` state are exposed through the public API.

### D — Denial of service (a Device that floods, drops, stalls, or truncates)

- Protocol reality: a point-to-point peer can always stop or corrupt
  communication; DoS by the peer is not fully defendable at the link (10.512 §7.8.2).
- Stack guarantees:
  - **No dynamic memory anywhere** (`grep` for `malloc`/`calloc`/`free` in `src/`
    and `include/` returns nothing). All port/controller state lives in
    caller-owned opaque storage with audited fixed budgets
    (`IOLINK_MASTER_PORT_STORAGE_SIZE`, `IOLINK_MASTER_CONTROLLER_STORAGE_SIZE` in
    `master.h`). A flooding Device cannot exhaust a heap because there is none.
  - **Bounded RX retry.** Checksum/short-frame failures retry at most twice
    (`rx_retry_count < 2U`, `src/master_port.c`) before surfacing an error and a
    counter increment; a Device injecting persistent bad checksums degrades the port
    to an observable error, not an unbounded loop.
  - **Response timeouts are explicit and scheduler-visible.** A dropped or truncated
    response is modelled as `IOLINK_MASTER_TICK_RESPONSE_TIMEOUT` /
    `iolink_master_on_timeout`, increments `response_timeouts`, and lets the
    caller-owned scheduler decide recovery — the core never blocks or sleeps
    (see [`PHY_BOUNDARY.md`](../PHY_BOUNDARY.md)). Truncated-frame and
    dropped-response recovery are covered by the fake-device harness
    (`tests/test_master_fake_device.c`).
  - **One bad port does not corrupt its siblings.** The controller isolates
    per-port state and returns the first negative per-port result without letting a
    failing port mutate others (`src/master_controller.c`,
    `iolink_master_controller_tick*`).

## 4. IEC 62443-4-2 requirement mapping (stack view)

Restating the *relevant* 10.512 clause 7 (Table 2) requirements as master-stack
claims. Rows Table 2 assesses "not relevant" (FR 1 identification, most of FR 2,
host/network-device requirements) are omitted for the reasons the guideline gives.

| Requirement (10.512 ref) | Stack claim | Anchor |
|---|---|---|
| CR 3.1 communication integrity (§7.4.2) | CRC6/checksum verified on every Device response before PD is exposed | `src/master_port.c` RX path, `../iolinki/src/crc.c` |
| CR 3.4 software/information integrity (§7.4.4) | Data Storage / block-param images verified by readback before being trusted | `iolink_master_verify_data_storage`, `iolink_master_write_parameter_block` |
| CR 3.5 input validation (§7.4.5) | Length/bounds/segment legality of every received frame, ISDU, event, and DS record enforced before use | `src/master_isdu.c`, `src/master_port.c`, `src/master_parameters.c` |
| CR 3.6 deterministic output (§7.4.6) | Response timeout / comm loss is an observable state and counter, not a hang | `iolink_master_on_timeout`, `iolink_master_get_diagnostics` |
| CR 3.7 error handling (§7.4.7) | Errors answered with the named result codes only; no internal state leaks through the public API | `include/iolinki_master/master.h` result enums |
| CR 5.1 network segmentation (§7.6.2) | Point-to-point by construction | protocol property |
| CR 7.3/7.4 backup and recovery (§7.8.4-5) | Data Storage upload/restore/verify sequencing with readback | `src/master_isdu.c` DS/block services |
| CR 7.6/7.7 least functionality (§7.8.6-7) | Services layer sits above cyclic transport; unused service calls are simply not invoked, and PD/ISDU sizes are caller-configured | `iolink_master_config_t`, services in `master.h` |

## 5. Gaps and integrator duties

Stated plainly, because a threat model that hides gaps is marketing:

1. **Not validated on real silicon.** All guarantees above are verified by local
   CTest and the fake-device / on-wire firmware-model harness, **not** against real
   IO-Link Devices. The physical 80µs WURQ wake pulse and the `t_WU`/`t_REN`/`TDMT`
   startup timing live in the PHY adapter and are **unverified on hardware** (see
   [`PHY_BOUNDARY.md`](../PHY_BOUNDARY.md), [`HARDWARE_VALIDATION.md`](../HARDWARE_VALIDATION.md)).
   A timing or adapter defect could admit or misclassify traffic this model assumes
   is rejected. Official IO-Link master conformance has **not** been run.
2. **Identity is inspection-level, not cryptographic, and `IDENTICAL` is partial.**
   The SerialNumber leg (ISDU 0x0015) that distinguishes `IDENTICAL` from
   `TYPE_COMP` is not wired. A VendorID/DeviceID-cloning Device is not detected.
3. **Physical protection is the countermeasure.** Per 10.512 §6 and 10.502,
   spoofing/tampering/disclosure on the C/Q wire are mitigated physically at
   SL-C 1. The master product's user documentation should carry that
   security-assessment recommendation.
4. **Master-product duties the stack cannot see:** the fieldbus/network uplink and
   its segmentation, firmware update and boot integrity of the master, secrets at
   rest, and the product's CRA risk assessment. No BLOB Transfer & Firmware Update
   profile is implemented here.
5. **10.512 D1.0.0-01 is a draft.** Claims cite the draft; the mapping is
   re-verified against the final release.

## 6. Verification

Claims here are regression-checked by the local CMocka/CTest suite and the
fake-device harness (see [`TESTING.md`](../TESTING.md)):

- Frame/checksum validation and retry: `tests/test_master_startup.c`,
  `tests/test_master_pd.c`, `tests/test_master_tick.c`
- ISDU parsing, segmentation, buffer bounds, write/readback:
  `tests/test_master_isdu.c`, `tests/test_master_isdu_public.c`
- Direct Parameter Page 1 and identity/inspection:
  `tests/test_master_parameters.c`
- Data Storage / block-param verify: `tests/test_master_isdu_public.c`,
  `tests/test_master_fake_device.c`
- Bad-checksum, dropped-response, and truncated-frame handling:
  `tests/test_master_fake_device.c`
- End-to-end against the real device stack over in-memory queues:
  `tests/test_master_real_iolinki_device.c`

*Maintenance rule:* any PR that changes a file cited as an anchor here must
re-verify the corresponding claim or update this document.
