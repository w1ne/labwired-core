# iolinki-master and the EU Cyber Resilience Act

*This page is orientation for master-product makers evaluating the stack. It is not
legal advice; your conformity assessment is yours.*

## You remain the manufacturer

Integrating `iolinki-master` — or any third-party stack — does not change your
status under Regulation (EU) 2024/2847 (the Cyber Resilience Act). For the IO-Link
master product you place on the EU market, CE marking, the EU Declaration of
Conformity, the ten-year technical-documentation retention, and the Article 14
reporting obligations stay entirely with you. What a stack supplier owes you is the
*foundation* your technical documentation builds on. That is what this package is.

Key dates: the CRA's vulnerability-reporting obligations apply from
**11 September 2026**; the full obligations from **11 December 2027**.

## What iolinki-master provides

| Deliverable | Where | Terms |
|---|---|---|
| SBOM per release (CycloneDX 1.6 + SPDX 2.3) | attached to every [tagged release](https://github.com/w1ne/iolinki-master/releases), **from `0.2`** | free, public |
| STRIDE threat model aligned to IO-Link guideline 10.512 (Master surface) | [`THREAT_MODEL.md`](THREAT_MODEL.md) | free, public |
| Coordinated disclosure + advisory process | [`SECURITY.md`](../../SECURITY.md) | free, public |
| CRA compliance statement mapping the stack to Regulation (EU) 2024/2847 Annex I, per stack release for your product context | commercial license package | commercial |
| Contractually agreed security updates over a defined support period | commercial license package | commercial |

The public artifacts let you verify our engineering rigor before you talk to us.
The commercial artifacts are the contract-grade documents your CRA technical file
and supplier-management process need.

## Zero third-party runtime dependencies

The master stack has **no third-party runtime dependencies**. It reuses only the
narrow `crc.c` / `frame.c` helper sources from the sibling `iolinki` device
repository, compiled directly into the master build (see `CMakeLists.txt` and
[`README.md`](../../README.md)); it does **not** link the device stack, a fieldbus
runtime, or any external library. The per-release SBOMs shipping from `0.2` state
this explicitly rather than by omission, so both source origins — this repository
and the pinned `iolinki` helper sources — appear in your bill of materials.

## Why a protocol stack is in scope at all

Commercially licensed software placed on the EU market is a "product with digital
elements" under the CRA. As the stack's supplier we carry manufacturer obligations
for the stack itself — which is why the SBOM, the disclosure process, and the
support-period commitment exist as maintained artifacts rather than sales material.
The stack is in the CRA's *default* (non-critical) class: conformity is
self-assessed, no notified body involved.

## The division of labor, concretely

**We cover (for the stack):** risk analysis of the master's attack surface — the
untrusted C/Q wire from a rogue Device, PHY adapter boundary, and ISDU/event/Data
Storage parsing (the [threat model](THREAT_MODEL.md)); input-validation and
integrity mechanisms with code anchors; SBOM; vulnerability handling and
advisories; security fixes over the support period.

**You cover (for your master product):** your product risk assessment; the
fieldbus/network uplink and its security; the PHY adapter and its wake-pulse/timing
integrity (the stack ships no hardware adapter — see the threat model's gaps and
[`PHY_BOUNDARY.md`](../PHY_BOUNDARY.md)); firmware update authenticity and boot
integrity of the master; physical-protection guidance in your user documentation;
your DoC, CE marking, and Article 14 reporting.

For commercial-package inquiries, email **andrii@shylenko.com** or open a
(non-security) discussion on the repository.
