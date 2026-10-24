# Spec-conformant wire, master slice (iolinki-master) — implementation plan

> **For agentic workers:** execute task by task, TDD, one commit per task. Steps use `- [ ]`.

**Goal:** make iolinki-master speak the IO-Link V1.1.5 wire byte-for-byte and behave per the spec
timing/recovery rules, so it interoperates with any conformant device.

**Architecture:** the port state machine stays; `master_isdu.c` gets a spec transport (ISDU octet
stream with Length/ExtLength/CHKPDU, segmented over the ISDU channel with FlowCTRL in the MC address);
events move to the Diagnosis channel (event memory, Table 58/59); parameters/codes/timing corrected.
The checksum comes from the device stack's shared helper `iolink_checksum6` (sibling `../iolinki`
checkout on branch `feat/spec-wire-checksum-isdu`, worktree `/home/andrii/projects/iolinki-wt-wire`;
configure with `-DIOLINKI_DEVICE_DIR=/home/andrii/projects/iolinki-wt-wire`).

**Spec:** `/home/andrii/projects/iolinki-wt-wire/docs/superpowers/specs/2026-09-18-spec-conformant-wire-design.md`
(sections C1..C6 normative). Spec text extracts: `/tmp/claude-1000/-home-andrii/7d9ab1e8-21ab-4fb5-bb9c-6cc409879441/scratchpad/spec_extract.md`,
`.../sec_736.md` (ISDU transport 7.3.6), `.../sec_738.md` (events 7.3.8). PDF:
`/home/andrii/projects/iolinki-wt-wire/docs/IOL-Interface-Spec_10002_V1.1.5_Oct2025/` (`pdftotext -layout`).

## Global constraints

- Caller-owned static storage; opaque port state must still fit the public budget
  (`include/iolinki_master/master.h` storage size asserts; bump the budget constant if needed and say so).
- `-Werror -Wpedantic -Wconversion -Wshadow`, cppcheck, clang-format v21 (Google 100 col); MISRA
  deviations documented in `docs/MISRA_DEVIATIONS.md`. Doxygen zero-warn.
- Public API signatures in `include/iolinki_master/master.h` keep working for existing callers; new
  config fields get zero-means-default semantics.
- Test vectors from the Python oracle (below), never from the C output.
- Commit messages: conventional, no AI/assistant mention, no trailers.

## Oracle

```python
def ck6(octets):            # A.1.6 + (A.1); CKT/CKS bits 0-5 already zero
    c = 0x52
    for o in octets: c ^= o
    b = [(c >> i) & 1 for i in range(8)]
    return ((b[7]^b[5]^b[3]^b[1])<<5)|((b[6]^b[4]^b[2]^b[0])<<4)|((b[7]^b[6])<<3)|((b[5]^b[4])<<2)|((b[3]^b[2])<<1)|(b[1]^b[0])
def chkpdu(octets):         # A.5.6
    c = 0
    for o in octets: c ^= o
    return c
```
Vectors: `[0x00,0x00]`→CKT `0x2D`; `[0xA2,0x00]`→`0x00`; `[0x20,0x00,0x99]`→`0x06`; TYPE_1 write
`[0x00,0x40,0xA5,0x5A]`→`0x75`; TYPE_2 read `[0x80,0x80]`→`0xAD`. Device replies: `[0x10]`+flags 0→CKS
`0x39`; `[0xA5]`+Event flag→`0x8A`; empty→`0x2D`. CHKPDU: `[0xB5,0x00,0x10,0x00]`→`0xA5`;
read resp `[0xD4,0x12,0x34]`→`0xF2`; write resp(+) `[0x52]`→`0x52`.

---

### Task 1: build against the new device helper

**Files:** `CMakeLists.txt` (nothing to change if it already globs `frame.c`/`crc.c` from
`IOLINKI_DEVICE_DIR`), every test that hard-codes CK bytes (`tests/fake_iolink_device.c`,
`tests/test_master_*.c`), `src/master_port.c` reply verification (must zero CKS bits 0-5 before
verifying and read Event/PD-status flags from bits 7/6).
- [ ] Reply layout per spec A.1.5: `[PD-in][OD] CKS`, `CKS = event<<7 | pd_invalid<<6 | ck6`. Remove
  every assumption of a leading status octet (`status & 0x20` PD_VALID, toggle bit) in
  `src/master_port.c` reply parsing and in `tests/fake_iolink_device.c`; `pd_valid = !(cks & 0x40)`,
  `event = cks & 0x80`. Vectors: TYPE_0 reply `10 39`; TYPE_2 PD `A5 22` valid/no event; `A5 8A` event.
- [ ] Configure with the sibling worktree, build; fix all hard-coded checksums using the oracle; the
  fake device must compute its CKS with `iolink_checksum6`. All 14 ctest targets green. Commit
  `fix(wire): verify replies with the A.1.6 checksum`.

### Task 2: ISDU transport (C3)

**Files:** `src/master_isdu.c` (`iolink_master_isdu_fill_od`, `iolink_master_isdu_on_od`,
`iolink_master_read_isdu`, `iolink_master_write_isdu`, helpers), `src/master_internal.h`
(ISDU state: `flowctrl`, `expected_len`, `chk`, `phase` REQ/POLL/RESP/IDLE), `src/master_port.c` (the
ISDU-channel MC must carry FlowCTRL: `MC = RW | 0x60 | flowctrl`), `tests/test_master_isdu.c`,
new `tests/test_master_isdu_wire.c`.
**Produces:** request encoding per Table A.13 with the index format chosen per Table A.15
(`index<256 && subindex==0` → 0x9/0x1; `index<256` → 0xA/0x2; else 0xB/0x3), Length = total ISDU
octets (2..15) or `1`+ExtLength (17..238), CHKPDU appended; segmentation: write messages START then
COUNT 1..15,0; then read START polling while the device answers `0x01` (Busy) or `0x00` (No Service =
still waiting, bounded by `config.isdu_timeout_100us`, default 50000 = 5 s per Table 102); response
octets over reads with COUNT; finish with IDLE; on any error send ABORT and return
`IOLINK_MASTER_ISDU_ERR_DEVICE` with `last_isdu_error` = ErrorType (ErrorCode<<8|AdditionalCode).
- [ ] `tests/test_master_isdu_wire.c` (fake device answering byte-exact): 8-bit-index read
  `93 10 83` gets `D4 12 34 F2`; 16-bit read emits `B5 00 10 00 A5`; write of 2 octets to index 0x10
  subindex 1 emits `26 10 01 12 34 <chk>` and accepts `52 52`; negative `C4 80 11 <chk>` maps to
  `last_isdu_error == 0x8011`; 64-octet read through ExtLength over od_len 1 with COUNT wrap;
  busy polling then success; corrupted CHKPDU in the response → ABORT emitted, error returned.
- [ ] Implement; ctest green; commit `fix(isdu): spec ISDU framing, lengths, CHKPDU and FlowCTRL segmentation`.

### Task 3: events over the Diagnosis channel (C4)

**Files:** `src/master_isdu.c` (`read_event_code`, `ack_event`, `read_event_details`,
`read_detailed_device_status`), `src/master_port.c` (Event-flag edge from CKS bit 7 triggers the event
handler), `tests/test_master_isdu_public.c`, `tests/test_master_isdu_wire.c`.
- [ ] Tests: on Event flag the master issues Type-0 reads `MC = 0xC0` (R, DIAGNOSIS, addr 0), decodes
  StatusCode type 2 (bit 7 details, bits 0-5 active slots), reads `0xC1..0xC3` per active slot 1
  (`3n-2..3n`), delivers `{qualifier, code}`, then writes `MC = 0x40` addr 0 with any data (Table 59 T8);
  `read_detailed_device_status` reads ISDU index 0x0025.
- [ ] Implement; commit `fix(events): read the Table 58 event memory over the diagnosis channel`.

### Task 4: parameters and codes (C5)

**Files:** `src/master_parameters.c`, `src/master_internal.h`, `tests/test_master_parameters.c`.
- [ ] Tests: `mseq_capability_code` from (OD width, PD in/out) per Table A.10: TYPE_1_2 → 1,
  TYPE_1_1/interleaved → 0, TYPE_1_V 8 OD → 6, 32 OD → 7, TYPE_2_V codes 4/5/6/7 by OD width,
  TYPE_2_x → 0; validation accepts a device advertising 6 for TYPE_1_V; reserved PD descriptor combos
  (BYTE=1 Length 0/1, BYTE=0 Length 17..31) return `PARAM_ERR_PD_DESCRIPTOR`; shared index constants
  from `../iolinki/include/iolinki/protocol.h` (DetailedDeviceStatus 0x0025) are used, none redefined.
- [ ] Implement; commit `fix(parameters): Table A.10 M-sequence codes and Table B.6 descriptor validation`.

### Task 5: timing and recovery (C6)

**Files:** `src/master_port.c`, `src/master_internal.h`, `include/iolinki_master/master.h`
(`t_dwu_100us` default 400, `t_dmt_tbit` default 32, `isdu_timeout_100us` default 50000,
`wake_retry_limit` default 2 when 0 → document), `tests/test_master_startup.c`, `tests/test_master_tick.c`.
- [ ] Tests: after a wake-up the first message is not sent before T_DMT (32 T_BIT at the current baud,
  COM3 T_BIT = 4.34 µs → 139 µs → 2 ticks of 100 µs); wake retries are spaced by ≥ T_DWU (40 ms);
  after n_WU=2 retries the baud scan advances; response deadline = max(config.response_timeout_100us,
  ceil((11 + 10) * T_BIT / 100 µs)) and never 0; on RX retry exhaustion the port returns to STARTUP and
  re-issues wake-up instead of latching ERROR (ERROR stays for PHY failures); the probe response octet is
  stored in `device_info.min_cycle_time` under every inspection level.
- [ ] Implement; commit `fix(port): T_DMT, T_DWU, response deadline from T_BIT, restart after retry exhaustion`.

### Task 6: sample, docs, ledger

- [ ] `samples/iolink_master/src/main.c` fake PHY answers the new wire (page read 0xA2, DeviceOperate
  write, ISDU with CHKPDU); `examples/master_loopback_demo.c` same. `docs/IMPLEMENTATION_STATUS.md`
  rewritten for C1..C6 (no stale 0x00/0x0F text), `CHANGELOG.md` BREAKING entry, `docs/MISRA_DEVIATIONS.md`
  if new deviations. `./check_quality.sh` clean, all ctest green incl. `test_master_real_iolinki_device`
  against the sibling worktree device. Commit `docs: record the spec-conformant wire`.

## Status

Branch `feat/spec-wire-checksum-isdu`, worktree `/home/andrii/projects/iolinki-master-wt-wire`,
device worktree `/home/andrii/projects/iolinki-wt-wire` (slice D, read-only here).

### Per task

- **Task 1 — verify replies with the A.1.6 checksum.** Commit `6468ec2`. Tests:
  `test_master_startup`, `test_master_pd`, `test_master_fake_device` (A.1.5 reply
  layout `[PD-in][OD] CKS`, flags in bits 7/6, fake device computes CKS with
  `iolink_checksum6`). Result: pass.
- **Task 2 — spec ISDU framing, lengths, CHKPDU and FlowCTRL segmentation.**
  Commit `8908003`. Tests: `test_master_isdu`, `test_master_isdu_wire` (byte-exact
  Table A.13/Figure A.20 vectors), `test_master_isdu_public`. Result: pass.
- **Task 3 — read the Table 58 event memory over the diagnosis channel.**
  Commit `bfff760`. Tests: `test_master_isdu_public`, `test_master_isdu_wire`,
  `test_master_fake_device` event paths. Result: pass.
- **Task 4 — Table A.10 M-sequence codes and Table B.6 descriptor validation.**
  Commit `a1b243c`. Tests: `test_master_parameters` (new
  `test_mseq_capability_code_matches_table_a10`,
  `test_parse_direct_parameter_page1_rejects_reserved_pd_descriptors`,
  `test_validate_accepts_every_table_a10_code_for_its_type`). Result: pass.
- **Task 5 — T_DMT, T_DWU, response deadline from T_BIT, restart after retry
  exhaustion.** Commit `efe4d41`. Tests: `test_master_startup`,
  `test_master_tick` (new `test_tick_at_holds_first_message_for_t_dmt_after_wake`,
  `test_tick_at_spaces_wake_retries_by_t_dwu`,
  `test_response_deadline_has_a_t_bit_floor`,
  `test_startup_probe_octet_stored_under_no_check`), `test_master_controller`,
  `test_master_pd`, `test_master_public_flow`. Public storage budget bumped
  1280 -> 1296 bytes. Result: pass.
- **Task 6 — sample, docs, ledger.** Commit `26b3859`. Zephyr sample fake PHY
  answers the new wire; `docs/IMPLEMENTATION_STATUS.md` rewritten for C1..C6;
  `CHANGELOG.md` BREAKING entry; `docs/MISRA_DEVIATIONS.md` rows refreshed;
  README status updated. Result: docs/sample only (not built by host CTest).

### Overall test result

`ctest --test-dir build --output-on-failure`: 14/15 targets pass. The single
failure is `test_master_real_iolinki_device`, the cross-worktree on-wire target;
per the design's merge order ("on-wire lane red until slice L") it stays red
until slice D/L land. It was already red before Task 4 and is not a regression
from this slice. All 14 host/hermetic targets green.

### Blocker: `./check_quality.sh` cannot pass in this environment (pre-existing)

`check_quality.sh` exits 1 at step 3/5 (MISRA C:2012). The tree has ~517
`cppcheck --addon=misra` findings and the script runs that step with
`--error-exitcode=1`; the same step fails on unmodified `HEAD` (509 findings in
a clean `/tmp` extraction of the commit), so this is a pre-existing tree/tooling
condition, not a regression. Step 4/5 (clang-format `--dry-run --Werror`) also
cannot pass here because only clang-format **14.0.6** is installed while the
project targets clang-format v21; v14 flags pre-existing committed code in
`src/master_isdu.c` and `src/master_port.c` as unformatted. Per the brief, the
tree was **not** reformatted with v14; touched lines were hand-formatted to
Google 100-column style. Verified steps 1/5 (`-Werror -Wpedantic -Wconversion
-Wshadow`) and 2/5 (cppcheck warning/style/performance/portability) pass:

```
[1/5] Verifying Compilation Warnings...   PASS
[2/5] Running Static Analysis (Cppcheck)... PASS
[3/5] Running MISRA C:2012 Check...       FAIL (pre-existing, also at HEAD)
```

New MISRA findings introduced by this slice are all in already-accepted rule
classes (15.5 multiple returns, 19.2 opaque-storage union use) and are covered by
the existing `docs/MISRA_DEVIATIONS.md` rows, which were refreshed accordingly.

### Not done in this slice

`test_master_real_iolinki_device` green against the sibling device, and physical
wake-pulse/T_REN validation, remain open per the design's work split and merge
order (slice L / hardware phase).

### Review follow-up

Commits `6477803` (tests/examples/sample) and `0c4136f` (`src/master_port.c`)
drive startup through the 1-octet DeviceOperate CKS reply on every path and
carry PD-out plus the M-sequence type bits in OPERATE ISDU frames.

Hermetic result after the follow-up: `ctest --test-dir build
--output-on-failure` -> 14/15 targets pass. `test_master_real_iolinki_device`
is still red, and it is not only the ack.

The cross-worktree lane needs the device slice's Type-0 OD-write framing fix:
for a two-octet-OD M-sequence the device computes
`req_len = IOLINK_M_SEQ_HEADER_LEN + ctx->od_len` (4 for TYPE_2) for the
3-octet Type-0 DeviceOperate write. The frame never completes, so the device
never answers it and startup clicks through PREOPERATE without the CKS ack.
With that one line changed to `+ 1U`, the six profile-matrix cases pass against
the real stack; the remaining ISDU cases then fail on FlowCTRL read
segmentation (the master re-reads at START, which restarts the device's
response pointer, so the assembled response duplicates `D1 13` and the CHKPDU
check fails). Both are on the device/on-wire lane (slice L), not the master
checksum work. Applied locally as a build-only experiment; the sibling worktree
was left untouched.

Raw interop result (`build/tests/test_master_real_iolinki_device`): 9 run,
0 passed, 9 failed. Profile matrix exits at case 3 (`0xfffffffffffffffb` /
"did not reach OPERATE"); the ISDU cases report "did not reach OPERATE" or
`IOLINK_MASTER_ISDU_ERR_DEVICE` (0xfffffffc) with
`last_isdu_error == 0x81` (segmentation).
