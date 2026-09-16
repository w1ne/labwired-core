# Vendored SEGGER RTT sources

`SEGGER_RTT.c` / `SEGGER_RTT.h` are unmodified copies from the Zephyr segger
module (west `modules/debug/segger`, SystemView 3.40 tree).

- SEGGER_RTT.c sha256: 03498dfeff9a52e7a809c6d62d2cc0a90057c5d9dcf6cf317e98b79be35b26df
- SEGGER_RTT.h sha256: b478e69d67e411b3e806c5003893df414127c6e9b08759241363fffbe47e37ab

License: SEGGER's 1-clause BSD notice is retained in the file headers.
`SEGGER_RTT_Conf.h` is LabWired-authored config shared by the bare-metal RTT
fixtures; its defaults can be overridden per build with `-D` (see the
blocking fixture).
