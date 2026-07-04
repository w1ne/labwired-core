# Security Policy

`iolinki-master` is a dual-licensed IO-Link **master** stack intended for
integration into industrial products. Security is handled as an engineering
discipline here, not a checkbox: the stack ships with a public
[threat model](docs/security/THREAT_MODEL.md) (STRIDE, aligned to the IO-Link
Security Design and Development Guideline, Order No. 10.512), per-release
machine-readable SBOMs (from `0.2`), a [CRA overview](docs/security/CRA.md), and
the coordinated-disclosure process below.

## Supported Versions

| Version | Security fixes |
| ------- | -------------- |
| Latest tagged release + `master` | ✅ free of charge |
| Older releases | Under a commercial support agreement only |

Security fixes are delivered as tagged releases with a changelog entry and, for
confirmed vulnerabilities, a GitHub security advisory. Commercial licensees can
contractually fix a support period per release — see the commercial
security-update terms.

## Reporting a Vulnerability

Please do **not** open a public issue for suspected vulnerabilities.

1. **Preferred:** GitHub private vulnerability reporting — use *Report a
   vulnerability* under the repository's **Security** tab.
2. Alternatively, contact the maintainer privately (contact details on the GitHub
   profile).

What to include: affected version/commit, the IO-Link frame, M-sequence, or ISDU
sequence (or code path) that triggers the issue, and impact as you understand it.
A proof of concept against the fake-device harness (`tests/fake_iolink_device.c`)
or the LabWired on-wire firmware model is ideal but not required.

**Response targets:**

- Acknowledgement within **72 hours**.
- Triage verdict (accepted / rejected / needs info) within **14 days**.
- Fix timeline agreed with the reporter at triage; critical issues in the frame,
  M-sequence, or ISDU parsing paths are prioritized ahead of all other work.

## Coordinated Disclosure

We ask reporters to withhold public disclosure until a fixed release is available.
In return we commit to: keeping the reporter informed, crediting them in the
advisory and changelog (unless they prefer otherwise), and not taking legal action
against good-faith research performed against your own or simulated hardware.

## CRA Readiness

For products placed on the EU market, Regulation (EU) 2024/2847 (Cyber Resilience
Act) applies — its vulnerability-reporting obligations from September 2026, its
full obligations from December 2027. As the supplier of a commercially licensed
stack we maintain the corresponding internal process: confirmed actively exploited
vulnerabilities in the stack are handled under the CRA notification regime (early
warning within 24 hours, notification within 72 hours) and communicated to
commercial licensees so they can meet their own Article 14 duties.

The stack has **zero third-party runtime dependencies** (it reuses only narrow
frame/CRC helper sources from the sibling `iolinki` checkout at build time). The
per-release SBOMs (CycloneDX 1.6 + SPDX 2.3) state this explicitly rather than by
omission. Integrating the stack does not transfer manufacturer obligations: device
makers remain responsible for their own conformity assessment, CE marking, and
reporting — see [`docs/security/CRA.md`](docs/security/CRA.md).
