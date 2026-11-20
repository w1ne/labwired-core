The first stack-frame smoke revision failed firmware compilation because its
naked handler used MOVW, which is unavailable on ARMv6-M / Cortex-M0. The
firmware build log and runner metrics are preserved here; this was a build
failure, not a simulator execution failure.
