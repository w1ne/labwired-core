"""Dependency-free controls; synthetic ELF/logs never qualify simulator behavior."""

import importlib.util
import pathlib
import os
import subprocess
import struct
import tempfile
import unittest

from test_core_identity_harness import literal_run


ROOT = pathlib.Path(__file__).resolve().parents[2]
VERIFIER = ROOT / "scripts/ci/cortex_m_oracle_evidence.py"


def fixture_bytes():
    data = bytearray(56)
    data[:7] = b"\x7fELF\x01\x01\x01"
    struct.pack_into("<HHI", data, 16, 2, 40, 1)
    struct.pack_into("<H", data, 40, 52)
    data[52:] = b"\x00\xbf\x00\xbf"
    return bytes(data)


def positive_log(oracle):
    test = "cpu_conformance_sim" if oracle == "nrf52_cpu_conformance" else "conformance_sim"
    if oracle == "f103_conformance":
        digest = "sim conformance digest: [C0DEF103, 0000001C, 0000001F, 00000000, 7D24A31B, 00000000, 00000000, DEADBEEF, 00000000, 00000000, 00000000]"
    else:
        cpu = oracle == "nrf52_cpu_conformance"
        title = "nRF52840 sim CPU-conformance digest:" if cpu else "nRF52840 sim conformance digest (16 words):"
        count = 9 if cpu else 16
        digest = title + "\n" + "\n".join(
            f"  [{i:02}] {'DONE' if i == 0 else 'field':<14} = 0x{0x52840D0E if i == 0 else i:08X}"
            for i in range(count)
        )
    return f"running 1 test\n{digest}\ntest {test} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n"


class EvidenceControls(unittest.TestCase):
    def setUp(self):
        self.assertTrue(VERIFIER.is_file(), "qualification verifier missing")
        spec = importlib.util.spec_from_file_location("oracle_evidence", VERIFIER)
        self.verifier = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.verifier)
        self.temp = tempfile.TemporaryDirectory(prefix="cortex-oracle-control-")
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)

    def fixture(self, data=None):
        path = self.root / "fixture.elf"
        path.write_bytes(fixture_bytes() if data is None else data)
        return path

    def log(self, text):
        path = self.root / "oracle.log"
        path.write_text(text)
        return path

    def test_real_shaped_elf_records_size_and_hash(self):
        evidence = self.verifier.verify_fixture(self.fixture())
        self.assertEqual(evidence["bytes"], 56)
        self.assertEqual(len(evidence["sha256"]), 64)

    def test_missing_elf_fails(self):
        with self.assertRaises(ValueError):
            self.verifier.verify_fixture(self.root / "missing")

    def test_empty_truncated_and_non_elf_fail(self):
        for data in (b"", b"\x7fELF", b"x" * 56):
            with self.subTest(data=data), self.assertRaises(ValueError):
                self.verifier.verify_fixture(self.fixture(data))

    def test_wrong_class_endian_architecture_or_header_fail(self):
        for offset, value in ((4, 2), (5, 2), (6, 0), (16, 1), (18, 62), (40, 0)):
            data = bytearray(fixture_bytes())
            data[offset] = value
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                self.verifier.verify_fixture(self.fixture(data))

    def test_header_without_payload_fails(self):
        with self.assertRaises(ValueError):
            self.verifier.verify_fixture(self.fixture(fixture_bytes()[:52]))

    def test_each_positive_oracle_records_asserted_done(self):
        for oracle in ("nrf52_conformance", "nrf52_cpu_conformance", "f103_conformance"):
            with self.subTest(oracle=oracle):
                evidence = self.verifier.verify_log(self.log(positive_log(oracle)), oracle, 0)
                self.assertEqual(evidence["oracle"], oracle)
                self.assertGreater(len(evidence["digest_words"]), 1)

    def test_fake_pass_without_digest_fails(self):
        text = "test conformance_sim ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n"
        with self.assertRaises(ValueError):
            self.verifier.verify_log(self.log(text), "nrf52_conformance", 0)

    def test_empty_missing_and_wrong_named_logs_fail(self):
        for text in ("", positive_log("nrf52_conformance").replace("test conformance_sim", "test other")):
            with self.subTest(text=text), self.assertRaises(ValueError):
                self.verifier.verify_log(self.log(text), "nrf52_conformance", 0)
        with self.assertRaises(ValueError):
            self.verifier.verify_log(self.root / "missing", "nrf52_conformance", 0)

    def test_missing_or_wrong_done_fails_each_oracle(self):
        for oracle in ("nrf52_conformance", "nrf52_cpu_conformance", "f103_conformance"):
            sentinel = "C0DEF103" if oracle == "f103_conformance" else "52840D0E"
            for replacement in ("00000000", ""):
                with self.subTest(oracle=oracle, replacement=replacement), self.assertRaises(ValueError):
                    self.verifier.verify_log(self.log(positive_log(oracle).replace(sentinel, replacement)), oracle, 0)

    def test_partial_digest_fails(self):
        text = positive_log("nrf52_cpu_conformance").replace("  [08] field          = 0x00000008\n", "")
        with self.assertRaises(ValueError):
            self.verifier.verify_log(self.log(text), "nrf52_cpu_conformance", 0)

    def test_skip_notice_fails_even_with_positive_output(self):
        with self.assertRaises(ValueError):
            self.verifier.verify_log(self.log("skip: firmware ELF not built\n" + positive_log("nrf52_conformance")), "nrf52_conformance", 0)

    def test_nonzero_exit_zero_filtered_or_failed_test_fails(self):
        good = positive_log("f103_conformance")
        for text, code in ((good, 1), (good.replace("1 passed", "0 passed"), 0), (good.replace("0 filtered", "1 filtered"), 0), (good.replace("0 failed", "1 failed"), 0)):
            with self.subTest(text=text, code=code), self.assertRaises(ValueError):
                self.verifier.verify_log(self.log(text), "f103_conformance", code)

    def test_literal_job_shell_has_complete_two_mode_route_and_fails_closed(self):
        workflow = (ROOT / ".github/workflows/core-identity.yml").read_text()
        self.assertIn("qualify_cortex_m_oracles:", workflow, "optional qualification route missing")
        source = literal_run(workflow, "Record exact Cortex-M oracle source")
        build = literal_run(workflow, "Build Cortex-M oracle firmware")
        run = literal_run(workflow, "Run Cortex-M digest oracles")
        mock_bin = self.root / "bin"
        mock_bin.mkdir()
        cargo = mock_bin / "cargo"
        cargo.write_text('''#!/usr/bin/env python3
import os,pathlib,sys
sys.path.insert(0,os.environ['CONTROL_DIR'])
from test_cortex_m_oracle_evidence import fixture_bytes,positive_log
args=sys.argv[1:]
if args[0]=='build':
 target=args[args.index('--target')+1]
 for i,arg in enumerate(args):
  if arg=='-p':
   p=pathlib.Path('target')/target/'release'/args[i+1]
   p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(fixture_bytes())
else:
 oracle=args[args.index('--test')+1]
 test='cpu_conformance_sim' if oracle=='nrf52_cpu_conformance' else 'conformance_sim'
 assert args[args.index('--test')+2]==test
 assert args[-3:]==['--','--exact','--nocapture']
 mode='scheduler' if '--features' in args else 'feature-off'
 if mode=='scheduler': assert args[args.index('--features')+1]=='labwired-core/event-scheduler'
 with open('calls','a') as f:f.write(mode+' '+oracle+'\\n')
 if os.environ.get('FAKE_PASS'):
  print('test '+test+' ... ok\\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s')
 else: print(positive_log(oracle))
''')
        cargo.chmod(0o755)
        git = mock_bin / "git"
        git.write_text('#!/bin/sh\n[ "$1" = rev-parse ] && [ "$2" = HEAD ] || exit 1\nprintf "%s\\n" "$MOCK_SOURCE_SHA"\n')
        git.chmod(0o755)
        sha = "6" * 40
        env = dict(os.environ, PATH=str(mock_bin) + os.pathsep + os.environ["PATH"],
                   CONTROL_DIR=str(VERIFIER.parent), EVIDENCE_VERIFIER=str(VERIFIER),
                   EXPECTED_SOURCE_SHA=sha, MOCK_SOURCE_SHA=sha)
        for script in (source, build, run):
            syntax = subprocess.run(["bash", "-n"], input=script, text=True, capture_output=True)
            self.assertEqual(syntax.returncode, 0, syntax.stderr)
            result = subprocess.run(["bash", "-c", script], cwd=self.root, env=env, text=True, capture_output=True, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(sha, (self.root / "oracle-evidence/source.json").read_text())
        for expected, actual in (("main", sha), (sha, "7" * 40)):
            bad = subprocess.run(["bash", "-c", source], cwd=self.root,
                                 env=dict(env, EXPECTED_SOURCE_SHA=expected, MOCK_SOURCE_SHA=actual),
                                 text=True, capture_output=True, timeout=15)
            self.assertNotEqual(bad.returncode, 0)
        self.assertEqual((self.root / "calls").read_text().splitlines(), [
            mode + " " + oracle for mode in ("feature-off", "scheduler")
            for oracle in ("nrf52_conformance", "nrf52_cpu_conformance", "f103_conformance")])
        for mode in ("feature-off", "scheduler"):
            for oracle in ("nrf52_conformance", "nrf52_cpu_conformance", "f103_conformance"):
                self.assertTrue((self.root / "oracle-evidence" / f"{mode}-{oracle}.exit").is_file())
        bad = subprocess.run(["bash", "-c", run], cwd=self.root, env=dict(env, FAKE_PASS="1"), text=True, capture_output=True, timeout=15)
        self.assertNotEqual(bad.returncode, 0)
        self.assertIn("FAIL-ORACLE-EVIDENCE", bad.stderr)
        fixture = self.root / "target/thumbv7em-none-eabi/release/firmware-nrf52840-conformance"
        fixture.unlink()
        bad = subprocess.run(["bash", "-c", run], cwd=self.root, env=env, text=True, capture_output=True, timeout=15)
        self.assertNotEqual(bad.returncode, 0)
        self.assertIn("missing evidence", bad.stderr)


if __name__ == "__main__":
    unittest.main()
