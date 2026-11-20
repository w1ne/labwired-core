# STM32F103 remeasurement interrupted

This is an interrupted CLI rebuild, not a platform simulation result. The
one-target `--only stm32f103` rerun was stopped by the operator after 15m58s
while `cargo build -p labwired-cli` was still compiling `labwired-core`; no
firmware build or simulator run started. The pipeline's transient `fail`
metrics and partial build log are retained here for audit. This interruption
does not replace the prior passing STM32F103 evidence or count as a platform
failure.
