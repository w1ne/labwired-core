# Known limitations

- The profile covers MCPWM0's timer/operator PWM path for output signals
  PWM0_OUT0A..2B. The smoke exercises PWM0A only. Timer capture, fault/trip
  handling, event/task matrix behavior, and the peripheral interrupt delivery
  path are not claimed.
- The C6 `MCPWM0.CLK.EN` bit is masked storage that gates timer advancement;
  PCR clock-tree frequency changes are not modeled. Other C6 tail registers
  are masked storage only.
- GPIO4 is routed by `FUNC4_OUT_SEL_CFG` to C6 matrix signal 87. IO_MUX electrical
  enforcement, analog levels, servo supply current, and physical movement are
  outside the simulation.
- UART0, GPIO8/RMT RGB output, and GPIO9 BOOT input are present in the system
  description, but this smoke does not exercise the onboard LED or button.
- Wi-Fi, Bluetooth LE, Zigbee/Thread, USB enumeration, LP core/deep sleep, and
  silicon parity are outside this profile.
