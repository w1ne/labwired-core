# A1 STEP/DIR stable core pin

The A1 Playground build can pin `941a6c5caf0c53b66aa5af9290e5ecba5972ec72`.
This backports the actual STM32 GPIO BSRR STEP/DIR observer from core PR #1906
onto the previously supported core baseline `7ae9652f717c428db3476caa2a8b0aa45abdcb8f`.

This merge records the backport as a core-main ancestor without changing the
current core source tree. The pin adds no chip descriptors. It avoids six
unrelated models that lack honest browser-run/hosted-compile support; it does
not pretend those models have been made runnable or change the 40-gap limit.

The A1 browser release must still prove firmware STEP edges, joint positions,
correct WASM/core ABI and deployed X/Y/Z/E mechanical movement. Later core
upgrades must resolve any newly introduced browser-coverage gaps normally.
