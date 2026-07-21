# nRF54L15 RADIO behavioural model — implementation spec

Scope: enough of the RADIO peripheral to make Zephyr's software Link-Layer RUN on
the advertising + single-connection path, at real timing, observable on the logic
analyzer. NOT RF physics / whitening / CRC math / encryption / coded PHY.

## Fixed facts
- RADIO base 0x5008_A000, size 0x1000. RADIO IRQ = 138 (RADIO_0_IRQn). NOT nRF52 layout.
- Anchor timer = TIMER10 @ 1 MHz (1 us/tick). Coarse anchor = GRTC CC[11]
  (HAL_CNTR_GRTC_CC_IDX_RADIO 11; ticker uses CC 10). Interconnect = DPPI + PPIB
  (LabWired has neither today — behavioural shortcut recommended).

## Critical SHORTS (54L bit map)
READY_START=0, DISABLED_TXEN=2, DISABLED_RXEN=3, PHYEND_DISABLE=19 (54L auto-disables
on PHYEND, NOT END — NRF_RADIO_SHORTS_TRX_END_DISABLE == PHYEND_DISABLE). These four
cover the whole advertise + connection path.

## Key registers (RADIO-relative)
Tasks: TXEN 0x000, RXEN 0x004, START 0x008, DISABLE 0x010. SUBSCRIBE_* 0x100+.
Events: READY 0x200, TXREADY 0x204, RXREADY 0x208, ADDRESS 0x20C, END 0x218,
PHYEND 0x21C (turnaround/disable trigger), DISABLED 0x220 (primary IRQ the LLL waits on),
CRCOK 0x22C. PUBLISH_* 0x300+.
Config: SHORTS 0x400, INTENSET00 0x488 / INTENCLR00 0x490, MODE 0x500, STATE(RO) 0x520,
FREQUENCY 0x708, TIFS 0x714, PCNF0 0xE20, PCNF1 0xE28 (MAXLEN read back), CRCSTATUS(RO)
0xE0C (=1 when scripted RX delivered), PACKETPTR 0xED0 (EasyDMA PDU ptr — load-bearing;
TX airtime from *(PACKETPTR+1) len). Everything else: benign store/readback stub.

## FSM
STATE: Disabled=0,RxRu=1,RxIdle=2,Rx=3,RxDisable=4,TxRu=9,TxIdle=0xA,Tx=0xB,TxDisable=0xC.
DISABLED --TXEN--> TxRu --(tx_ru)--> READY/TXREADY; if READY_START auto START.
TxIdle --START--> Tx --(AA airtime)--> ADDRESS --(payload+crc)--> END,PHYEND.
  on PHYEND: if PHYEND_DISABLE -> TxDisable --(ramp-down)--> DISABLED -> EVENTS_DISABLED,
  IRQ 138 if INTENSET00.DISABLED. on DISABLED: if DISABLED_RXEN auto RXEN (turnaround).
Symmetric for RX. Every 0->1 EVENTS_* -> PeripheralTickResult.fired_events (for DPPI) +
result.irq if enabled. radio_is_idle() reads STATE==0 so ramp-down MUST reach DISABLED.

## Timing (us guest time; x128 -> cycles)
Ramp-up 1M: fast TX ~41us / RX ~40us; default TX ~141 / RX ~140 (LLL selects via TIMING/
MODECNF0). Airtime 1M: preamble 8 + AA 32 + header + len*8 + CRC 24 (1us/bit) — MUST be
proportional to len (adv PDUs ~130-350us). tIFS ~150us turnaround (SW-switch on 54L: LLL
arms TIMER10 compare; model the EFFECT off the SHORTS bits, not the plumbing — auto-issue
opposite ramp-up ~T_IFS after PHYEND). Fake instant: whitening/CRC/RSSI/PLL/FREQUENCY.

## Anchor (hard part)
GRTC CC[11] compare -> (PPIB) -> TIMER10 START; TIMER10 CC[0]=remainder_us -> (DPPI ch9) ->
RADIO TXEN/RXEN. Companion: ch13 END/PHYEND->TIMER10 CAPTURE; ch12 TIMER10 hcto -> RADIO
DISABLE (this is what times out an RX window into the void). SHORTCUT for M1: minimal DPPI
channel table (store CHIDX from SUBSCRIBE/PUBLISH, route published events to subscribed task
addrs via the lib.rs route_ppi_events/mmio_writes seam) OR hardcode channels 8,9,12,13.
DEPENDS ON the GRTC absolute-compare (>=) fix — a strict == would drop every anchor.

## Medium
Advertising needs NO peer: TX into void, RX window hits HCTO timeout -> DISABLE -> next
channel/interval. Connection needs a scripted peer: CONNECT_IND into RX PACKETPTR during the
post-adv RX window (latch ADDRESS/END/CRCOK), then once/interval an empty LL data PDU
(LLID=1,len=0) ping-pong. Plug a `ScriptedBleMedium` in at the Rx-START boundary (BabbleSim-lite,
no RF), matching the SimInput stimulus pattern.

## Staged plan
1. Advertise-only (no peer): FSM+SHORTS+events+IRQ + timing wheel + GRTC CC[11]->TXEN + HCTO->
   DISABLE via minimal DPPI table. ~60% of the work.
2. Scripted-peer connection: ScriptedBleMedium, CONNECT_IND + empty-PDU ping-pong, CRCOK,
   RX->TX tIFS turnaround. Validates lll_conn/lll_peripheral.
3. BabbleSim-style medium (later): two RADIO instances, real PDU exchange, filtering, enc.

## Risks
DPPI/PPIB cross-domain routing (use behavioural collapse); timing tolerances (ramp-up + airtime
+ tIFS must be real, sub-us can be constant; GRTC-anchor->TXEN latency is the delicate number);
GRTC >= compare fix must be live; confirm TIMER10 actually ticks at 1 MHz on 54L (prescaler 5);
TIFS_HW Kconfig unknown -> key off SHORTS bits actually written.

## Logic-analyzer tagging (BLE connection-event timeline)
Anchor = GRTC CC[11] fire. TX window = TXREADY..PHYEND (tag FREQUENCY channel + PDU len).
RX window = RXREADY..END/HCTO (mark ADDRESS = peer detected vs HCTO timeout). ADDRESS marker =
on-air SOP. PHYEND marker = EOP/turnaround pivot. tIFS gap = shaded PHYEND..next READY (~150us).
DISABLED marker = event end / IRQ 138 (tag whether NVIC pended). STATE lane = enum step signal.
