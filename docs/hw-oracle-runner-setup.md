# HW Oracle Self-Hosted Runner Setup

This document covers how to configure a Linux machine as the `esp32s3-zero`
self-hosted runner that executes `.github/workflows/hw-oracle.yml`.

---

## 1. Hardware Requirements

- Linux machine (x86-64 or ARM64) with USB port
- **ESP32-S3-Zero** attached via USB (VID:PID `303a:1001`)
- Reliable power — flaky USB hubs will cause intermittent test failures

---

## 2. Software Prerequisites

### udev rule (USB access without root)

Create `/etc/udev/rules.d/99-esp32s3-zero.rules`:

```
SUBSYSTEM=="usb", ATTR{idVendor}=="303a", ATTR{idProduct}=="1001", MODE="0666", GROUP="dialout"
```

Reload and verify:

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger
lsusb | grep 303a          # should show the board
ls -l /dev/ttyACM*         # should be group dialout, mode 0660 or 0666
```

Add the runner user to the `dialout` group:

```bash
sudo usermod -aG dialout $USER   # re-login required
```

### OpenOCD

Install version `0.12.0+dev-ge4c49d8` or newer with ESP32-S3 support.
The LabWired HW oracle uses the `esp32s3.cfg` target configuration.

```bash
# Verify
openocd --version
# Expected: Open On-Chip Debugger 0.12.0+dev-ge4c49d8 (or newer)
```

### Xtensa toolchain

The Rust/Xtensa toolchain must be present at:

```
~/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/
```

Install via `espup`:

```bash
cargo install espup
espup install          # installs the esp toolchain under ~/.rustup/toolchains/esp
```

### Rust stable (for host-side compilation)

```bash
rustup toolchain install stable
```

---

## 3. Register the GitHub Actions Runner

Follow the [official GitHub docs](https://docs.github.com/en/actions/hosting-your-own-runners/managing-self-hosted-runners/adding-self-hosted-runners).
The critical step is assigning the `esp32s3-zero` label so the workflow targets
this machine.

Brief example:

```bash
mkdir ~/actions-runner && cd ~/actions-runner

# Download the runner package (check GitHub for the latest URL)
curl -o actions-runner-linux-x64.tar.gz -L \
  https://github.com/actions/runner/releases/download/v2.323.0/actions-runner-linux-x64-2.323.0.tar.gz
tar xzf actions-runner-linux-x64.tar.gz

# Configure — paste the token from the repo Settings > Actions > Runners page
./config.sh \
  --url https://github.com/ORG/REPO \
  --token YOUR_REGISTRATION_TOKEN \
  --labels esp32s3-zero \
  --name my-esp32s3-runner

# Install as a systemd service
sudo ./svc.sh install
sudo ./svc.sh start
```

Verify the runner appears as **Idle** under
`Settings > Actions > Runners` in the GitHub repository.

---

## 4. Testing Locally Before Relying on CI

Run the HW oracle test suite directly on the machine with the board attached:

```bash
# Confirm board is present
lsusb | grep '303a:1001'

# Run all hw-oracle tests (they are marked #[ignore] to skip in sim)
cargo test -p labwired-hw-oracle --features hw-oracle -- --ignored --test-threads=1
```

`--test-threads=1` is required: multiple tests competing for OpenOCD on the
same USB device will fail with `Address already in use 6666`.

---

## 5. Troubleshooting

**`libusb: Access denied` / `Permission denied on /dev/ttyACM0`**
- Check the udev rule loaded: `udevadm info /dev/ttyACM0 | grep GROUP`
- Confirm the runner user is in the `dialout` group: `groups $USER`
- Unplug and replug the board after fixing the rule

**`Address already in use (os error 98)` on port 6666**
- A previous OpenOCD instance is still running:
  ```bash
  pkill openocd
  ```
- If it recurs, add a `pkill openocd || true` pre-step to the workflow

**`PoisonError: Mutex poisoned`**
- A prior test panicked while holding the board mutex
- The process must be restarted; `cargo test` will do this automatically on the
  next run since each `cargo test` invocation is a fresh process

**Board not detected mid-run (`lsusb` shows nothing)**
- USB cable or hub issue — use a direct port or powered hub
- Check `dmesg | tail -30` for USB disconnect events

**`openocd: command not found` on the runner**
- OpenOCD is not in the runner user's `PATH`; add to `~/.bashrc` or set
  `PATH` explicitly in the workflow step environment
