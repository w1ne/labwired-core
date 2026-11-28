#!/bin/sh
# Rebuild idle_hook.elf from idle_hook.S (clang + the rust-lld rustup ships).
set -e
cd "$(dirname "$0")"
LLD=$(ls "$(rustc --print sysroot)"/lib/rustlib/*/bin/rust-lld | head -1)
clang --target=thumbv7em-none-eabi -mcpu=cortex-m4 -c -o idle_hook.o idle_hook.S
cat > idle_hook.ld <<'LD'
MEMORY { FLASH : ORIGIN = 0x08000000, LENGTH = 64K
         RAM : ORIGIN = 0x20000000, LENGTH = 64K }
SECTIONS {
  .vectors : { KEEP(*(.vectors)) } > FLASH
  .text : { *(.text*) } > FLASH
  .bss (NOLOAD) : { *(.bss*) } > RAM
}
LD
"$LLD" -flavor gnu -nostdlib -e reset --build-id=none -z max-page-size=16 -T idle_hook.ld \
  --strip-all -o idle_hook.elf idle_hook.o
rm -f idle_hook.o idle_hook.ld
