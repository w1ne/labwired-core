# Initial STM32L151 smoke attempt

The CLI and firmware built, but the smoke did not execute firmware. The first
narrowed descriptor used `64K`/`8K` sizes, which the chip descriptor parser
rejects. The attempt is retained as a failure record; the corrected descriptor
uses byte-suffixed sizes.
