# at91rm9200 — 2026-11-08 build_cli failure (host toolchain)

First `--only at91rm9200` run after the profile adopted the ARM926EJ-S model.
It never reached the simulator: `cargo build -p labwired-cli --profile
native-system --features qemu-arm` failed in `unicorn-engine-sys`'s build
script because bindgen's clang could not find `stdbool.h` on this host
(`build_cli-error.log`). The root filesystem was also at 100% use.

This is a host-toolchain failure, not a model result. The rerun set
`BINDGEN_EXTRA_CLANG_ARGS='-isystem /usr/lib/gcc/x86_64-linux-gnu/13/include'`
(the workaround used for the A20/A9 imports) and built into a separate
`CARGO_TARGET_DIR` under `/dev/shm`.

A second attempt with the bindgen fix also failed at `build_cli`: the shared
root filesystem was at 0 bytes free and sccache could not create its temp dir
(`No space left on device (os error 28)` compiling `bit-set`). The next rerun
disabled the rustc wrapper and pointed `TMPDIR` at `/dev/shm`.
