# NUCLEO-H753ZI evidence before the main sync

This directory preserves the passing H753 M7 FPU smoke generated before
`origin/main` was merged into the Renode lane. The main sync changed Cortex-M7
FPU and exception code, so the target was rerun against the merged tree. The
pre-sync metric, logs, JUnit report, simulator result, and snapshot remain here.
