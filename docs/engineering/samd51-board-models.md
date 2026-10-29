# SAMD51 board models

Models for running a complete SAMD51 board, ported from
`CrispStrobe/labwired-core` (MIT). `configs/chips/atsamd51.yaml` and
`configs/chips/atsamd51-pybadge.yaml` declare them. Construction goes through
one cold builder with fixed instance addresses and IRQs; the DAC's board
reference levels and the I2C master's SCL rise time are its only config
options.

| Type | Register slice | Not modelled |
| --- | --- | --- |
| `samd51_oscctrl` | Reset-enabled DFLL48M; DPLL0/1 with `DPLLRATIO`, `DPLLCTRLB` (GCLK reference only), `DPLLSYNCBUSY` and `DPLLSTATUS`. A DPLL locks only once its reference channel (`GCLK PCHCTRL[1 + n]`) is enabled. | Crystals, DFLL closed loop, lock time, jitter, interrupts, events (refused). |
| `samd51_gclk` | The existing `SamGclk` model plus SAMD51 `GENCTRL[0..11]` at `0x20 + 4n` (GENCTRL0 resets to DFLL, enabled). Same type, so SERCOM `gclk_id` gating keeps working. | Generator output pins, `GCLK_IN`, chaining. |
| `samd51_tc` | TC0..TC5: 8/16-bit counters with NFRQ/MFRQ/NPWM/MPWM tops, compare-match and overflow flags/IRQs, CTRLB commands, READSYNC, CCBUF/PERBUF applied at the UPDATE (wrap). The count rate is `f_TC / f_CPU` resolved from the live clock tree. Waveform outputs WO0/WO1 for up-counting NPWM and MPWM WO1 are published edge-exact onto pads routed to PMUX function E (`DRVCTRL.INVEN` inverts). Each wrap latches one overflow DMA request (`take_dma_trigger`); the opt-in `samd51_dmac_board` consumes it. | NFRQ/MFRQ and MPWM WO0 outputs, down-counting outputs (drive low), capture, events, 32-bit pairing. |
| `samd51_eic` | Sixteen channels, rise/fall/both/high/low sensing, flags and IRQ lines 12..27. A pad edge reaches channel n only while the pad is muxed to the EIC (PMUXEN, function A) and the EIC is bus- and generic-clocked and enabled. PINSTATE follows the delivered level. | Filtering, debouncing, asynchronous timing, NMI, events (refused). |
| `samd51_dac` | Channel 0 conversions from DATA0 writes while `CTRLA.ENABLE`, `DACCTRL0.ENABLE` and the generic clock (`PCHCTRL[42]`) hold; `STATUS.READY0`. Each conversion is recorded with its engine cycle. With the board's `vrefa_mv` and `vddana_mv` declared, `output_millivolts(code)` is the nominal level DATA / 4095 x VREF for the selected REFSEL. | Gain, offset and linearity error, settling, load, the internal reference, refresh, dithering, channel 1, differential mode, events, interrupts, DATABUF. |
| `samd51_pac` | The access-error flags drivers clear before use (no access errors are modelled: flags read 0). | Write protection, events, the PAC interrupt (refused). |
| `samd51_tcc` | TCC0..TCC4 counters (24-bit TCC0/1, 16-bit TCC2..4) at the prescaled generic clock, PER top, CC0..CCn matches, CTRLB commands, and the SAMD51 lines: line 0 for OVF/TRG/CNT/ERR/faults, line 1 + x for MCx. | Waveforms, CCBUF/PERBUF, dithering, faults, capture, patterns, events, DMA. |
| `samd51_adc` | ADC0/ADC1 single-ended software-triggered conversions of AINn against GND with the VDDANA references (INTVCC1, INTVCC0), 8/10/12-bit, `floor(V * 2^n / Vref)`. A conversion completes on the next bus tick only while the ADC is enabled and clocked and the input's pad is on PMUX B. Input voltages are set by the host (`set_input_millivolts`). | Analog front end, averaging, correction, windowing, differential mode, events, DMA. |
| `sam_sercom_spi` (timing) | With `gclk_id` resolving on the live clock tree, a byte takes 16 * (BAUD + 1) core clocks at `f_CPU / f_GCLK`, the fraction carried across back-to-back bytes; otherwise nominal core-clock cycles. The pad configuration is unchanged. | Pad and edge timing, slave mode. |
| `samd51_sercom_i2cm` | SERCOM I2C master (declared as the Wire SERCOM: SERCOM2 on the PyBadge, SERCOM5 on the Metro M4): 7-bit addressing, writes, smart-mode reads, repeated start, STOP, MB/SB/RXNACK against attached `I2cDevice` slaves. An operation completes on the next bus tick only while the SERCOM's bus and core clocks run and a PAD0 (SDA) and PAD1 (SCL) pad are muxed to it (ATDF pad table). With the core clock resolving, an operation lasts whole SCL periods of `10 + BAUD + BAUDLOW + f_core * t_rise` core clocks, with the rise time from the optional `scl_rise_ns` (default 0): START plus nine bits per address, nine more for a read's first byte, nine per byte, one per STOP. A device's `I2cDevice::clock_stretch_ns` is added once per byte it acknowledges or sends (twice for a read address, which carries the first byte). With `CTRLA.LOWTOUTEN`, a hold beyond the SCL low time-out (25 ms, the earliest of the datasheet's 25-35 ms) ends the operation there: MB or SB as normal, STOP sent, `STATUS.LOWTOUT` and `BUSERR` with `INTFLAG.ERROR`; a START written during a STOP follows it. An open read stays on the bus tick (a smart-mode DATA read issues the next byte). `busy()` and `completed()` expose operation progress to hosts. | Rise-time waveform, arbitration, the other bus timeouts (MEXTTOEN, SEXTTOEN, INACTOUT), 10-bit/high-speed, DMA, slave mode (refused). |
| `samd51_sercom_usart` | Opt-in (the board descriptors keep their console UARTs): `sam_sercom_usart` with the SAMD51 interrupt layout: DRE, TXC, RXC and the remaining flags on four consecutive NVIC lines, reported as level sources (a DATA read can deassert RXC). Optional `tx_port_base`/`tx_bit`/`rx_port_base`/`rx_bit`/`pad_mux`/`txpo`/`rxpo`: a byte reaches the TX sink, and host bytes reach the receiver, only while that pad is muxed and selected by TXPO/RXPO (latched on the bus tick). With the pads and `gclk_id` declared, bytes are paced at the baud from BAUD and SAMPR (arithmetic and fractional) on the live clock tree: frames of start, data, parity and stop bits, a DATA holding register before the shift register (DRE, TXC), host bytes a frame apart into a two-byte receive buffer (BUFOVF beyond). On an `event-scheduler` build the SERCOM wakes itself at each frame end while a byte is in flight. | As `sam_sercom_usart`; external-clock and reserved SAMPR settings move no byte when paced. |
| `samd51_usb_device_banks` | Opt-in USB device endpoint banks at the fixed USB instance (`0x41000000`, IRQ 80) with the MCLK AHBMASK/APBBMASK clocks. Endpoint descriptor banks and the CTRLA.RUNSTDBY bit. | Host mode, full transfer engine, interrupts beyond the bank window. |
| `samd51_dmac_board` | Opt-in byte/half-word board DMAC at the fixed DMAC instance. Descriptor table + write-back; a channel paced by a peripheral `take_dma_trigger` (TC overflow) moves samples into a destination such as DAC DATA. | Full DMAC priority/CRC/events; multi-block linking beyond the board profile. |

`sam/clock_tree.rs` resolves nominal generator, peripheral-channel and CPU
frequencies from GCLK/OSCCTRL/MCLK registers on the bus. A source that is not
running resolves to `None`, never to a guessed frequency.

## Engine changes these models rely on

- **SAM pads get a `PortId`.** `PinPort::port_name_prefix` lets a port family
  name its ids; SAM PORT groups are `porta`/`portb`. External pad edges now
  reach edge sinks on SAM boards (`gpioX` naming is unchanged).
- **Edge sinks are bus-ticked.** After an external edge reaches a sink, the bus
  refreshes that sink's bus-tick membership, so a sink that defers the edge to
  `tick_with_bus` (the EIC) is serviced.
- **SAM PORT AF routing.** `selected_function` decodes `PINCFG.PMUXEN`/`PMUX`,
  and `bind_timer_output_pad` binds SAM pads on function E; only pads with a
  bound route consult it.
- **`Peripheral::timer_output_family`** names a timer's output family, so the
  SAMD51 TC pad wiring needs no downcast.
- **`I2cDevice::clock_stretch_ns`** (default 0) lets a slave declare how long
  it holds SCL low per byte; only a controller that models bus timing adds it.
- **`Peripheral::on_system_reset`.** AIRCR.SYSRESETREQ (and a peripheral-requested reboot) calls it on every model so SAM peripherals return to their power-on register state with the CPU.
- **No bit-band on the SAMD51.** `chip_has_bit_band` returns false for
  `atsamd51*` parts: their APB-C/D bridges map TC4/TC5, SERCOM4..7, the DAC and
  QSPI inside the would-be alias window. Every other chip is unchanged.

## Limits

Nominal frequencies only; bus timing follows registers and nominal clocks. These models make a natural boot and the qualified
peripheral paths possible; behaviour beyond the slices above is not modelled,
and none of this is a physical or timing qualification.
