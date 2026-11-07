#!/usr/bin/env python3
"""Check the SPI2 wire of ch32fun's examples_v20x/spidmatest.

Reads the `logic_edges` block of a `labwired test` result.json captured with
`--watch-gpio gpiob:13 --watch-gpio gpiob:15` (SPI2 SCK / MOSI) and decodes
MOSI on every rising SCK edge (CPOL 0, CPHA 0, MSB first, 16-bit frames), with
the framing counted from the first SCK edge of the run. The capture must hold
that edge: the script refuses a trimmed ring.

What the unmodified example has to put on the wire:

- 256 words of 0xAAAA: the first lap of DMA1 channel 5 over `sendbuff`,
  which the example memsets to 0xAA before it stores CNTR = 256;
- then only 0x0AAF: DMA1_Channel5_IRQHandler refills the half the channel
  has just sent (HT -> words 0..127, TC -> words 128..255) with 0x0AAF, so
  every word of the second and later laps is 0x0AAF;
- nothing else: not the 0xACAC of the earlier memset (the channel is enabled
  with CNTR = 0 while the buffer holds it), and no word before the first lap.

A trailing partial word cut by the stop limit is reported and ignored.
The input may be gzipped. Exits non-zero on the first mismatch.
"""
import bisect
import gzip
import json
import sys

WORDS_PER_LAP = 256
FIRST_LAP = 0xAAAA
REFILL = 0x0AAF


def main(path):
    opener = gzip.open if path.endswith(".gz") else open
    with opener(path, "rt") as f:
        edges = json.load(f)["logic_edges"]
    chans = {(c["peripheral"], c["pin"]): c for c in edges["channels"]}
    sck, mosi = chans[("gpiob", 13)], chans[("gpiob", 15)]
    for c in (sck, mosi):
        if c["edge_count"] != len(c["transitions"]):
            sys.exit(f"FAIL: {c['func']} ring trimmed ({len(c['transitions'])} of "
                     f"{c['edge_count']} edges); rerun with a shorter max_cycles")

    rises = [e["cycle"] for e in sck["transitions"] if e["value"] == 1]
    m_cycles = [e["cycle"] for e in mosi["transitions"]]
    m_values = [e["value"] for e in mosi["transitions"]]

    def mosi_at(cycle):
        i = bisect.bisect_right(m_cycles, cycle)
        return m_values[i - 1] if i else mosi["initial"]

    whole = len(rises) // 16
    words = []
    for w in range(whole):
        bits = rises[16 * w:16 * w + 16]
        words.append((bits[0], int("".join(str(mosi_at(c)) for c in bits), 2)))
    tail = len(rises) - 16 * whole

    if len(words) < 2 * WORDS_PER_LAP:
        sys.exit(f"FAIL: only {len(words)} whole words; need two laps")
    for i, (cycle, value) in enumerate(words):
        want = FIRST_LAP if i < WORDS_PER_LAP else REFILL
        if value != want:
            sys.exit(f"FAIL: word {i} (lap {i // WORDS_PER_LAP + 1}) at cycle {cycle}: "
                     f"0x{value:04X}, expected 0x{want:04X}")

    starts = [c for c, _ in words]
    spans = sorted({b - a for a, b in zip(starts, starts[1:])})
    bit = sorted({b - a for a, b in zip(rises, rises[1:]) if b - a <= 2})
    print(f"first SCK edge at cycle {rises[0]}; SCK rising-edge spacing within a word: {bit} cycles")
    print(f"word-to-word spacing: {spans} cycles")
    print(f"lap 1: words 0..255 = 0x{FIRST_LAP:04X} x{WORDS_PER_LAP}")
    print(f"laps 2+: words 256..{len(words) - 1} = 0x{REFILL:04X} x{len(words) - WORDS_PER_LAP} "
          f"({(len(words) - WORDS_PER_LAP) / WORDS_PER_LAP:.2f} laps)")
    if tail:
        print(f"trailing partial word: {tail} bits, cut by the stop limit (ignored)")
    print("PASS")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
