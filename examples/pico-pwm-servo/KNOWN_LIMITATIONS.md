# Known limitations

- The smoke uses PWM slice 0 channel A on GP16 only. It checks register values
  and transitions, not a measured frequency or pulse width from physical
  hardware.
- The SG90-compatible servo is a signal observer. Its VCC is not powered in
  the twin, and the smoke does not assert decoded servo angle or mechanical
  movement. Use a separate regulated supply with common ground on a physical
  setup.
- The hosted verifier assumes ideal VDD and derives device time from the
  declared 125 MHz `cpu_hz`; PLL changes are not tracked. See the recorded
  [verification gaps](evidence/hosted-verification.md).
- The RP2040 chip descriptor's `tbman` window is a zero-answering stub. This
  firmware does not access it. USB, ROM/UF2 startup, USB CDC, and a physical
  board capture are not part of this profile.
