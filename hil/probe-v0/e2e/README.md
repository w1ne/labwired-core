# Probe V0 — compute-link e2e test

`probe_e2e.py` is the first end-to-end gate from the [risk-first spec](../design.md):
prove a controlling host can reach the Orange Pi and drive the SPI1 master that the
GW1N-9 FPGA will connect to. It talks to **live hardware** over SSH.

## What it asserts

1. The Pi is reachable over SSH.
2. `/dev/spidev1.0` exists (SPI1 overlay applied).
3. The `spi1` master is registered in the kernel.
4. A 4-byte ioctl transfer through `spidev` succeeds and returns the right length.

With nothing wired to MISO the transfer reads an idle bus (all `0x00`) — the test
asserts the *path*, not the data. Two ways to tighten it as hardware arrives:

- **Loopback:** jumper MOSI→MISO and set `PROBE_EXPECT_LOOPBACK=1` — the master must
  read back exactly what it sent.
- **FPGA:** once the GW1N SPI slave answers, extend with a device-id/timestamp read
  (Stage 1 pass condition in the spec).

## Running

```bash
export PROBE_PI_HOST=root@192.168.1.247        # current DHCP lease of labwired-probe
export PROBE_SSH_KEY=~/.ssh/id_ed25519
# lab-specific: the Pi is wired, the dev host is on WiFi behind a 2-node mesh that
# isolates the two, so hop through the router:
export PROBE_SSH_JUMP='sshpass -p <router-pw> ssh -o PreferredAuthentications=password -o PubkeyAuthentication=no root@192.168.1.1 nc %h %p'

python3 probe_e2e.py          # standalone report, exit 0/1
pytest probe_e2e.py -v        # or under pytest (skips if PROBE_PI_HOST unset)
```

The Pi's DHCP lease is not pinned; find its current IP from the router lease table
(`cat /tmp/dhcp.leases | grep labwired-probe`).

## Config (env)

| var | default | meaning |
| --- | --- | --- |
| `PROBE_PI_HOST` | *(required)* | `user@ip` of the Pi |
| `PROBE_SSH_KEY` | ssh default | private key path |
| `PROBE_SSH_JUMP` | *(none)* | ProxyCommand string (router hop) |
| `PROBE_SPI_BUS` / `PROBE_SPI_CS` | `1` / `0` | selects `/dev/spidev1.0` |
| `PROBE_SPI_HZ` | `1000000` | transfer clock |
| `PROBE_EXPECT_LOOPBACK` | *(unset)* | require RX == TX (MOSI↔MISO jumpered) |
