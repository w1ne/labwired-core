# HIL — Hardware-in-the-Loop oracle runners

The `_hw` / `_diff` oracle tests cross-check the simulator against **real
silicon** over a debug probe. Run on a developer bench they prove the model
once; run here — on a self-hosted runner with the board permanently attached —
they prove it **on every push**, so the "silicon-verified" anchor stops decaying
the moment the bench is unplugged.

This directory is the registry + runner; the workflow is
`.github/workflows/core-hil.yml`. It is **inert until a runner is registered** —
it only triggers on manual dispatch and only schedules onto self-hosted runners
labelled `hil` + a board label, so nothing here affects the hosted PR/CI path.

## Layout

| File | Purpose |
|------|---------|
| `boards.json` | board registry — one entry per board, `status: active` puts it under HIL |
| `run-hil.sh` | manifest-driven runner; works on a runner or standalone on a bench |
| `../.github/workflows/core-hil.yml` | the inert workflow (matrix from `boards.json`) |

## Smoke-test on a bench (no runner needed)

With a board plugged in and `openocd` installed:

```bash
hil/run-hil.sh stm32f103-bluepill   # or: hil/run-hil.sh all
```

This is exactly what the runner job executes — so a green bench run means the
runner will be green too.

## Bring up a runner (the Mac server)

1. **Toolchain** (once):
   ```bash
   brew install openocd        # provides openocd + libusb
   curl https://sh.rustup.rs -sSf | sh
   ```
   Confirm `openocd --version` and `cargo --version` work in a fresh shell.

2. **Attach the board** and confirm the probe enumerates:
   ```bash
   # ST-Link should appear as 0483:374b
   system_profiler SPUSBDataType | grep -i st-link
   hil/run-hil.sh stm32f103-bluepill   # should pass before you register the runner
   ```

3. **Register the GitHub Actions self-hosted runner** (repo → Settings → Actions
   → Runners → New self-hosted runner → macOS), and give it the **labels for
   every board attached to that host**, plus the shared `hil` label:
   ```
   ./config.sh --url https://github.com/w1ne/labwired-core \
               --token <TOKEN> \
               --labels hil,stm32f103 \
               --name mac-396-hil
   ./run.sh    # or install as a service: ./svc.sh install && ./svc.sh start
   ```
   A board's job runs on `["self-hosted", "hil", "<runner_label>"]`, so the
   runner must carry that board's `runner_label` (here `stm32f103`). One host can
   serve several boards — give it all their labels.

4. **Go live**: set the matching `boards.json` entry to `status: active`, and
   uncomment the `schedule:` trigger in `core-hil.yml` so the anchor refreshes
   without a manual dispatch. (Make the `hil` check **required** in branch
   protection only once a runner is reliably online — otherwise it would block
   merges whenever the bench is offline.)

## Adding a board

Add an entry to `boards.json` (copy an existing one), point `test`/`features` at
its oracle bank, give it a `runner_label`, and attach it to a host carrying that
label. `status: planned` lists it here without creating jobs; flip to `active`
when its runner is up. Every supported chip should end up with an entry.
