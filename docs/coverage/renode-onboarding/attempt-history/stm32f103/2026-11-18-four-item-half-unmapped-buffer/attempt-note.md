# Four-item circular DMA buffer address omission

The first strict run of the new smoke assertion booted the imported target and
reached the check, which failed with `HALF1=1`, `LAP=1`, and `HT_REPEAT=1`,
but `HALF_DATA=0` and `FULL_DATA=0`. The captured buffer was
`00000000,00000000,00000000,00000000` while CNDTR and interrupt flags advanced.

The firmware had configured DMA1 channel 1's peripheral address and count but
omitted its memory address. It was therefore not writing the result buffer.
This attempt is preserved as a measured smoke failure; the fix programs
`DMA1_CMAR1` with the four-item buffer address before enabling the channel.
