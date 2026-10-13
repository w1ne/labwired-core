"""Opt-in isolation, private-app pin and reviewed module reproduction contracts."""
import json
from pathlib import Path
import subprocess
import os
import unittest
ROOT=Path(__file__).resolve().parents[2]
class BasketCi(unittest.TestCase):
    def validate(self, sha, token):
        env={**os.environ,'LABWIRED_BASKET_APP_COMMIT':sha,'LABWIRED_BASKET_APP_READ_TOKEN':token}
        return subprocess.run(['python3',str(ROOT/'scripts/ci/validate-basket-ci-inputs.py')],env=env,text=True,capture_output=True)
    def test_exact_full_commit_and_private_read_secret_required(self):
        for sha,token in [('', ''),('main','token'),('a'*39,'token'),('g'*40,'token'),('A'*40,'token'),('a'*40,'')]:
            r=self.validate(sha,token);self.assertNotEqual(r.returncode,0);self.assertNotIn('token',r.stdout)
        r=self.validate('a'*40,'private-read-credential');self.assertEqual(r.returncode,0,r.stderr);self.assertNotIn('private-read-credential',r.stdout+r.stderr)
    def test_native_job_covers_joint_contracts_and_busybox_read_regression(self):
        text=(ROOT/'.github/workflows/core-qemu-arm.yml').read_text()
        for item in ['rv1103_basket_loader','rv1103_rk3x_i2c','dw_uart_timeout','cosim_canonical_basket','test_prepare_rv1103_basket.py','busybox']:
            self.assertIn(item,text)
    def test_native_preparation_installs_reviewed_actual_compiler_before_tests(self):
        text=(ROOT/'.github/workflows/core-qemu-arm.yml').read_text()
        job=text.split('  native-system:',1)[1].split('  linux-acceptance:',1)[0]
        pins=json.loads((ROOT/'examples/luckfox-pico-mini-a-linux/basket/pins.json').read_text())
        package_version=pins['compiler_version'].split('(',1)[1].split(')',1)[0]
        compiler_package='gcc-arm-none-eabi='+package_version
        self.assertIn(compiler_package,job,'the two actual helper builds require the pinned compiler, not a mock or skipped test')
        self.assertIn('runs-on: ubuntu-24.04',job,'the reviewed compiler package belongs to the fixed Ubuntu 24.04 runner')
        self.assertLess(job.index(compiler_package),job.index('test_prepare_rv1103_basket.py'))
    def test_filtered_cargo_lanes_require_actual_positive_execution(self):
        text=(ROOT/'.github/workflows/core-qemu-arm.yml').read_text()
        guards=[line.strip() for line in text.splitlines() if "grep -Eq '^test result: ok" in line]
        self.assertEqual(len(guards),2,'unfiltered harness roster cannot prove filtered execution')
        for guard in guards:
            pattern=guard.split("'",2)[1]
            for output,expected in [('test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 7 filtered out;\n',1),('test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out;\n',0)]:
                r=subprocess.run(['grep','-Eq',pattern],input=output,text=True,capture_output=True)
                self.assertEqual(r.returncode,expected,output)
    def test_opt_in_job_pins_private_app_before_export_before_compile(self):
        text=(ROOT/'.github/workflows/core-qemu-arm.yml').read_text();job=text[text.index('  basket-acceptance:'):]
        self.assertIn("inputs.basket_acceptance",job);self.assertIn('ubuntu-24.04',job);self.assertIn('gcc-arm-none-eabi=15:13.2.rel1-2',job)
        self.assertIn('repository: w1ne/labwired',job);self.assertIn('secrets.LABWIRED_BASKET_APP_READ_TOKEN',job);self.assertIn('ref: ${{ inputs.basket_app_commit }}',job)
        self.assertLess(job.index('repository: w1ne/labwired'),job.index('export-native-runtime.ts'))
        self.assertLess(job.index('export-native-runtime.ts'),job.index('cargo build'))
        self.assertIn('--profile native-system',job);self.assertIn('if: always()',job);self.assertNotIn('prepare-rv1103-modules.py --',job)
    def test_bundle_is_exact_historical_reviewed_identity(self):
        import hashlib
        pins=json.loads((ROOT/'examples/luckfox-pico-mini-a-linux/basket/pins.json').read_text());base=ROOT/'examples/luckfox-pico-mini-a-linux/basket/reviewed-modules'
        for name,pin in {'module-manifest.json':pins['module_receipt'],**pins['modules']}.items():
            b=(base/name).read_bytes();self.assertEqual(len(b),pin['bytes']);self.assertEqual(hashlib.sha256(b).hexdigest(),pin['sha256'])
