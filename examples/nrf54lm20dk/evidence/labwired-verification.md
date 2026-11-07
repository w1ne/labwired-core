# LabWired model verification — nRF54LM20 DK LED0 smoke

Checked 2026-11-04 UTC with `labwired_verify` on the `nrf54lm20a` target.
The firmware's MMIO application code was submitted with a generic default
interrupt-vector table because the hosted Rust scaffold enables its device
feature; the local `cortex-m-rt` build supplies its own generic table. Both
builds run the same LED0/UARTE20 application logic.

- Status: **model_verified**
- Oracle: `gpio_edges` on `gpio1:22`, minimum 16 transitions
- Observed: **17 transitions**; serial contained
  `NRF54LM20DK_LED_SMOKE_OK`
- Run stopped when the GPIO oracle passed.

The verifier reports that VDD is assumed to be an ideal rail because its
diagram has no routed power net. This leaves power-on and brown-out reset
behavior unverified; it does not block the GPIO edge verdict. The local smoke
also gates the serial marker and GPIO transitions; its committed logs and
simulator output are in this evidence directory.
