# Nokia 5110 invaders fixture

`nokia5110-invaders-lab.elf` is the playground's demo firmware for the Nokia
5110 starter: release asset `demo-nokia5110-invaders-lab.elf` of
`firmware-demos-v15` (sha256
`9642009b61d1eca547ce93f2e06c2f9569d29badbfc64446ed571f6ed46e17a0`), built from
`examples/nokia5110-invaders-lab` as a Rust release build with DWARF. Its line
table matches that example's `src/main.rs`.

The source-debug tests use it because the optimiser inlined most of `main`'s
loop: `game.step(paddle)`, `lcd_frame(&FB)`, `delay(20_000)` and `measure()`
have no line-table row of their own, only `DW_TAG_inlined_subroutine` call
sites. Only `game.render(&mut FB)` (line 508) is a real call.
