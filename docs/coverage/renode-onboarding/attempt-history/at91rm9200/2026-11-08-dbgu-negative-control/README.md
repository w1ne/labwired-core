# at91rm9200 — 2026-11-08 DBGU negative control

Same fixture (`tests/fixtures/at91rm9200-dbgu-smoke.elf`) and script, run by the
same `native-system` CLI build against a copy of the committed chip profile
with the `uart0` (DBGU) entry removed (`chip-without-dbgu.yaml`).

Result: FAIL 0/2 checks, 0 retired instructions, `memory_violation` at
`0xFFFFF200` (the first `DBGU_CR` store, PC `0x0C`); no UART bytes captured
(`result.json`). The passing run therefore depends on the DBGU window, not on
a zero-answering stub or an unmapped-access fallback.
