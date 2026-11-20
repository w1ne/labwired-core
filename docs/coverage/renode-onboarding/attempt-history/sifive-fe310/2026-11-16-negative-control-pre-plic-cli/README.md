# Negative control: pre-PLIC CLI, same firmware and script

- CLI: `target/debug/labwired` built 2026-11-16 00:21 UTC from `a636e5541`
  (RV32 CLINT present, no RV32 PLIC: nothing decodes 0x0C00_0000).
- Firmware: the 2026-11-16 PLIC smoke, SHA-256
  `d059fb122ea7850807813d8c77463edb9be0ed40dc00b64f4e031db14034444c`.
- Result: FAIL 1/4 checks at step 42990, `Memory access violation at
  0xc200000` (the first PLIC threshold store in `plic_uart0_masked_probe`).
  Every CLINT line before it still passes.
