"""Source-owned Windows canary release-kind regression; no network calls."""
import json
from pathlib import Path
import subprocess
import unittest
ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT/'scripts/ci/cli-release-tag.py'
class CliReleaseTag(unittest.TestCase):
    def select(self, releases):
        return subprocess.run(['python3', str(SCRIPT)], input=json.dumps(releases), text=True, capture_output=True)
    def test_firmware_latest_is_not_a_cli_release(self):
        r = self.select([{'tag_name':'firmware-demos-v15'}, {'tag_name':'v0.25.0'}, {'tag_name':'v0.24.0'}])
        self.assertEqual(r.returncode, 0, r.stderr); self.assertEqual(r.stdout.strip(), 'v0.25.0')
    def test_semver_order_and_draft_prerelease_guards(self):
        r = self.select([{'tag_name':'v0.9.0'}, {'tag_name':'v0.10.0'}, {'tag_name':'v8.0.0','draft':True}, {'tag_name':'v9.0.0','prerelease':True}, {'tag_name':'v10.0.0-beta'}, {'tag_name':'firmware-demos-v99'}])
        self.assertEqual(r.returncode,0,r.stderr); self.assertEqual(r.stdout.strip(),'v0.10.0')
    def test_missing_cli_and_invalid_shape_fail_explicitly(self):
        for releases in ([{'tag_name':'firmware-demos-v15'}], {'tag_name':'v0.25.0'}, []):
            r=self.select(releases); self.assertNotEqual(r.returncode,0); self.assertEqual(r.stdout,'')
    def test_single_release_array_is_accepted_without_relaxing_object_guard(self):
        release={'tag_name':'v0.25.0'}
        r=self.select([release]); self.assertEqual(r.returncode,0,r.stderr); self.assertEqual(r.stdout.strip(),'v0.25.0')
        r=self.select(release); self.assertNotEqual(r.returncode,0); self.assertEqual(r.stdout,'')
    def test_windows_workflow_uses_source_owned_selector(self):
        text=(ROOT/'.github/workflows/install-canary.yml').read_text(); start=text.index('  cli-windows:'); end=text.index('  cli-old-distros:', start) if '  cli-old-distros:' in text[start:] else text.index('  chips:',start)
        leg=text[start:end]; self.assertIn('cli-release-tag.py',leg); self.assertIn('releases?per_page=100',leg); self.assertNotIn('releases/latest',leg)
        self.assertIn('ConvertTo-Json -AsArray',leg,'pwsh must preserve release-array shape at cardinality one')
