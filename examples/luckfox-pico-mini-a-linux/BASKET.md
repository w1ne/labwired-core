# Reduced native Linux basket, local dogfood only

The actual joint receipt `/tmp/labwired-rv1103-task4c-live-guest-v3-20261012.json` SHA256 `b341e8431bed1f3799d9cfd33ac9aa3048e948670f53cb1a3cdf65e6d4c7513d` is PASS / ACCEPTED, physical UNAVAILABLE. It binds historical HEAD `491fb2f` plus frozen dirty sources, CLI `44bf188b` and backend `fcef17db`; later integration/main/current-export checks do not retroactively change that run. Two fresh Linux 6.18.56-0-lts / armv7l PID1 fixtures pass eight phases; six powered phases each 19 I2C events / 110 TX and 154 RX bytes / seven behavioral commands; two off phases zero inner I2C/four AT TX/zero RX/ENXIO errno 6. Original failed v1/v2, base boot and prelaunch preparation receipts remain distinct.

Direct zImage executes through the QEMU-derived Renode tlib core and actual Machine/SystemBus/CosimSession. Shared-kit ADXL345 I2C3 and behavioral ESP32-C3 UART4 connect to one reduced persistent power plant. Raw 5 V maps to a synthetic execution VDD proxy, separate from raw 3.3 V / effective external 3.3 V with functional 2.7 V domain. External cut and latch reset are explicit. Actual plant short holds 100 ms / 2.4 million fuel / zero retirements/empty complete interval; clear alone remains held. Healthy reset and fresh CPU/bus/controllers/components recovery retain the SAME plant/battery/global time, without SOC rewind. Register retention applies only within an external-cut fixture.

There are 890 actual 100 ms plant steps, SOC 0.8 → 0.792586514519413, 2,138,314,240 cycles and 1,538,795,070 retirements. Actual wall time 644.13082329 s vs 89 functional-clock seconds means **no 1× claim**. Fixed functional 24 MHz / paced UART is not physical timing. No vendor BSP / SD / ROM / RF / ESP firmware / full RV1103 / all 50 silicon / physical acceptance. Browser/default remain refused. Existing app power-only viewer stays separate; `examples/smart-basket/render-native-demo.ts` renders this recorded joint evidence locally, never Linux in browser.

## Clean preparation and local build

Use Ubuntu 24.04 and exact `gcc-arm-none-eabi=15:13.2.rel1-2`, `device-tree-compiler=1.7.0-2build1`, Node 22.22.0 and Rust 1.95.0. The helper checks its exact compiler first version line; moving tool versions must fail rather than change pins.

```bash
python3 scripts/prepare-rv1103-linux.py --cache /path/to/fresh-cache --output /path/to/fresh-base
python3 - /path/to/fresh-cache <<'PY'
import importlib.util, pathlib, sys
spec=importlib.util.spec_from_file_location('modules','scripts/prepare-rv1103-modules.py')
modules=importlib.util.module_from_spec(spec); spec.loader.exec_module(modules)
cache=pathlib.Path(sys.argv[1])
modules.base.cached_apk(cache,modules.DEV['url'],modules.DEV['sha256'],modules.DEV['size'])
modules.cached_source(cache)
PY
python3 scripts/prepare-rv1103-basket.py --cache /path/to/fresh-cache \
  --base-artifacts /path/to/fresh-base \
  --module-artifacts examples/luckfox-pico-mini-a-linux/basket/reviewed-modules \
  --output /path/to/fresh-basket
```

The [exact reviewed module bundle](basket/reviewed-modules/README.md) is HISTORICAL. Its whole original manifest is pinned `f5fab55c…`, then both KO bytes, ELF/vermagic/CRC/source/package/preparer guards are rechecked. It is not a fresh CI module rebuild. A fresh module producer has fresh path/tool/log observations and thus a different whole-manifest hash; do not overwrite its provenance or weaken validation. Base/basket helper/DTB/archive preparation above is fresh and still guest NOT_RUN. Rebuilding modules separately is supported by `prepare-rv1103-modules.py` but does not replace this reviewed original bundle automatically.

Export from an explicit pinned app source BEFORE compile, using that app's Node dependencies and `examples/smart-basket/tsconfig.json`:

```bash
node /path/to/app/node_modules/tsx/dist/cli.mjs --tsconfig /path/to/app/examples/smart-basket/tsconfig.json \
  /path/to/app/examples/smart-basket/export-native-runtime.ts --core "$PWD" \
  --out "$PWD/examples/luckfox-pico-mini-a-linux/basket-native-runtime"
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo build --locked -p labwired-cli --bin labwired \
  --profile native-system --features qemu-arm-system,event-scheduler
./target/native-system/labwired rv1103-basket --artifacts /path/to/fresh-basket \
  --receipt /path/to/fresh-receipt.json --max-cycles 4000000000 --max-fuel 4000000000 \
  --max-retirements 2000000000 --wall-seconds 3600 --console-bytes 65536
```

The unchanged shipped profile is opt3 / thin LTO / debug0 / overflow checks ON / panic unwind. The CLI requires matching build checkout, app source and export bundle, checking compiler-captured critical source and current authorities before model startup. It is not an installed standalone portable binary. For current metadata-only checks, use small positive bounds (e.g. 2,400,032 cycles / 2,400,032 fuel / 32 retirements / 10 wall seconds) and a fresh receipt path: expected FAIL / NOT_RUN (unaccepted) after held startup, POR release and 32 real zImage instructions is separate from full historical PASS. No behavioral loader/controller/helper/model/CLI changes are justified by that preflight alone.

`core-qemu-arm.yml` keeps the strict base gate and default/browser isolation. Joint guest CI is opt-in only, with explicit full 40-hex lowercase `basket_app_commit` and `LABWIRED_BASKET_APP_READ_TOKEN` repository-read secret for PRIVATE w1ne/labwired. Core GITHUB_TOKEN is not assumed to have access; missing pin/auth fails clearly. The pinned app is checked out and exported before compile. Complete console, success/failure receipt, source/input/CLI/backend hashes and preparation are archived even on failure. This task does not dispatch the workflow, publish npm/release, deploy or claim hardware acceptance.
