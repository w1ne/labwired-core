# Chip list

Every chip LabWired models: **523 parts** — **114 public** (open models in [labwired-core](https://github.com/w1ne/labwired-core)) and **409 private** (commercial models). 4 retired part(s) are kept in the table marked excluded.

The table is generated from the live catalogues by `scripts/generate_chip_catalog.py` and refreshed in CI, so it cannot drift from the models. Private parts show a name and a coverage level only — their descriptor contents stay internal.

## How to read the coverage level

| Level | Meaning |
|---|---|
| **L2 verified** | Conformance level 2: register-level match to silicon *and* a behaviour gate passes on real firmware. |
| **L1 modelled** | The peripheral estate answers at its own addresses *or* a behaviour gate passes — one of the two. |
| **L0 estate** | Modelled and boots, with no register or behaviour gate yet. |
| **modelled** | The chip descriptor exists and drives firmware; no conformance level recorded yet. |
| **verified (P0-P5)** | Private model that passed all six evidence gates: descriptor, runnable example, smoke on a real image, instruction audit, and a committed report. |
| **modelled (private)** | Private model that runs firmware but has not cleared every gate. |
| **excluded** | Retired on purpose — obsolete or not buyable. |

> **T1** is the tier-1 happy/sad-path matrix (`T1 8✓` = 8 subsystems passing). **reg** is the count of cold-reset registers reproduced from a silicon capture. Both are public-side evidence.

<div class="lw-chips" markdown="0">
  <div class="lw-chips__controls">
    <input id="lw-chip-q" type="search" placeholder="Filter by name…" aria-label="Filter chips by name" />
    <label>Family <select id="lw-chip-family">
      <option value="">all families</option>
      <option value="AVR">AVR</option>
      <option value="Arm Cortex-M">Arm Cortex-M</option>
      <option value="Other">Other</option>
      <option value="PIC16">PIC16</option>
      <option value="PIC18">PIC18</option>
      <option value="PIC24 / dsPIC33">PIC24 / dsPIC33</option>
      <option value="RISC-V">RISC-V</option>
      <option value="Xtensa LX6">Xtensa LX6</option>
      <option value="Xtensa LX7">Xtensa LX7</option>
    </select></label>
    <label>Vendor <select id="lw-chip-vendor">
      <option value="">all vendors</option>
      <option value="Digilent">Digilent</option>
      <option value="Espressif">Espressif</option>
      <option value="LiteX">LiteX</option>
      <option value="Microchip">Microchip</option>
      <option value="NXP">NXP</option>
      <option value="Nordic Semiconductor">Nordic Semiconductor</option>
      <option value="OpenHW">OpenHW</option>
      <option value="Other">Other</option>
      <option value="Raspberry Pi">Raspberry Pi</option>
      <option value="Rockchip">Rockchip</option>
      <option value="STMicroelectronics">STMicroelectronics</option>
      <option value="Silicon Labs">Silicon Labs</option>
      <option value="Texas Instruments">Texas Instruments</option>
    </select></label>
    <label>Coverage <select id="lw-chip-coverage">
      <option value="">all coverage</option>
      <option value="L0 estate">L0 estate</option>
      <option value="L1 modelled">L1 modelled</option>
      <option value="L2 verified">L2 verified</option>
      <option value="modelled">modelled</option>
      <option value="modelled (private)">modelled (private)</option>
      <option value="verified (P0-P5)">verified (P0-P5)</option>
      <option value="excluded">excluded only</option>
    </select></label>
  </div>
  <p class="lw-chips__count" id="lw-chip-count">Showing 523 of 523 chips.</p>
  <table class="lw-chips__table">
    <thead><tr>
      <th scope="col">Chip</th>
      <th scope="col">Vendor</th>
      <th scope="col">Family</th>
      <th scope="col">Package</th>
      <th scope="col">Coverage</th>
      <th scope="col">Model</th>
    </tr></thead>
    <tbody>
      <tr class="lw-chip" data-name="arty-litex-vexriscv" data-family="RISC-V" data-vendor="Digilent" data-coverage="L1 modelled" data-status="supported" data-search="arty-litex-vexriscv digilent risc-v  l1 modelled">
        <td><code>arty-litex-vexriscv</code></td>
        <td>Digilent</td>
        <td>RISC-V</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="esp32c3" data-family="RISC-V" data-vendor="Espressif" data-coverage="L2 verified" data-status="supported" data-search="esp32c3 espressif risc-v  l2 verified">
        <td><code>esp32c3</code></td>
        <td>Espressif</td>
        <td>RISC-V</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L2 verified</span> <span class="lw-dim">T1 8✓</span> <span class="lw-dim">reg 380</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="esp32c6" data-family="RISC-V" data-vendor="Espressif" data-coverage="L1 modelled" data-status="supported" data-search="esp32c6 espressif risc-v  l1 modelled">
        <td><code>esp32c6</code></td>
        <td>Espressif</td>
        <td>RISC-V</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span> <span class="lw-dim">T1 12✓</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="esp32" data-family="Xtensa LX6" data-vendor="Espressif" data-coverage="L1 modelled" data-status="supported" data-search="esp32 espressif xtensa lx6  l1 modelled">
        <td><code>esp32</code></td>
        <td>Espressif</td>
        <td>Xtensa LX6</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span> <span class="lw-dim">T1 9✓</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="esp32s3" data-family="Xtensa LX7" data-vendor="Espressif" data-coverage="L1 modelled" data-status="supported" data-search="esp32s3 espressif xtensa lx7  l1 modelled">
        <td><code>esp32s3</code></td>
        <td>Espressif</td>
        <td>Xtensa LX7</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span> <span class="lw-dim">T1 9✓</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="esp32s3-zero" data-family="Xtensa LX7" data-vendor="Espressif" data-coverage="L1 modelled" data-status="supported" data-search="esp32s3-zero espressif xtensa lx7  l1 modelled">
        <td><code>esp32s3-zero</code></td>
        <td>Espressif</td>
        <td>Xtensa LX7</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="esp32s3devkitc1" data-family="Xtensa LX7" data-vendor="Espressif" data-coverage="verified (P0-P5)" data-status="supported" data-search="esp32s3devkitc1 espressif xtensa lx7 esp32-s3-devkitc-1 board verified (p0-p5)">
        <td><code>esp32s3devkitc1</code></td>
        <td>Espressif</td>
        <td>Xtensa LX7</td>
        <td class="lw-dim">ESP32-S3-DevKitC-1 board</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="litex-vexriscv" data-family="RISC-V" data-vendor="LiteX" data-coverage="L1 modelled" data-status="supported" data-search="litex-vexriscv litex risc-v  l1 modelled">
        <td><code>litex-vexriscv</code></td>
        <td>LiteX</td>
        <td>RISC-V</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="at90usb1286" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="at90usb1286 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>at90usb1286</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="at90usb646" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="at90usb646 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>at90usb646</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega128" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega128 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega128</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega1280" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega1280 microchip avr tqfp-100 verified (p0-p5)">
        <td><code>atmega1280</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-100</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega1281" data-family="AVR" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="atmega1281 microchip avr  l1 modelled">
        <td><code>atmega1281</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atmega1284p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega1284p microchip avr tqfp-44 verified (p0-p5)">
        <td><code>atmega1284p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-44</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega16" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega16 microchip avr dip-40 verified (p0-p5)">
        <td><code>atmega16</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega162" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega162 microchip avr dip-40 verified (p0-p5)">
        <td><code>atmega162</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega165" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega165 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega165</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega165p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega165p microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega165p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega168" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega168 microchip avr dip-28 verified (p0-p5)">
        <td><code>atmega168</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega168p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega168p microchip avr dip-28 verified (p0-p5)">
        <td><code>atmega168p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega169" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega169 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega169</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega2313" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega2313 microchip avr dip-20 verified (p0-p5)">
        <td><code>atmega2313</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega2560" data-family="AVR" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="atmega2560 microchip avr  l1 modelled">
        <td><code>atmega2560</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atmega2561" data-family="AVR" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="atmega2561 microchip avr  l1 modelled">
        <td><code>atmega2561</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atmega32" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega32 microchip avr dip-40 verified (p0-p5)">
        <td><code>atmega32</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega324p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega324p microchip avr tqfp-44 verified (p0-p5)">
        <td><code>atmega324p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-44</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega325" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega325 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega325</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega3250" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega3250 microchip avr tqfp-100 verified (p0-p5)">
        <td><code>atmega3250</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-100</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega3250p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega3250p microchip avr tqfp-100 verified (p0-p5)">
        <td><code>atmega3250p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-100</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega325p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega325p microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega325p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega328" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega328 microchip avr dip-28 verified (p0-p5)">
        <td><code>atmega328</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega328p" data-family="AVR" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="atmega328p microchip avr  l1 modelled">
        <td><code>atmega328p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atmega329" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega329 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega329</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega3290" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega3290 microchip avr tqfp-100 verified (p0-p5)">
        <td><code>atmega3290</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-100</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega3290p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega3290p microchip avr tqfp-100 verified (p0-p5)">
        <td><code>atmega3290p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-100</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega329p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega329p microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega329p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega48" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega48 microchip avr dip-28 verified (p0-p5)">
        <td><code>atmega48</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega48p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega48p microchip avr dip-28 verified (p0-p5)">
        <td><code>atmega48p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega64" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega64 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega64</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega640" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega640 microchip avr tqfp-100 verified (p0-p5)">
        <td><code>atmega640</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-100</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega644" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega644 microchip avr dip-40 verified (p0-p5)">
        <td><code>atmega644</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega644p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega644p microchip avr dip-40 verified (p0-p5)">
        <td><code>atmega644p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega645" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega645 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega645</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega6450" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega6450 microchip avr tqfp-100 verified (p0-p5)">
        <td><code>atmega6450</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-100</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega649" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega649 microchip avr tqfp-64 verified (p0-p5)">
        <td><code>atmega649</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega6490" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega6490 microchip avr tqfp-100 verified (p0-p5)">
        <td><code>atmega6490</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">TQFP-100</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega8" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega8 microchip avr dip-28 verified (p0-p5)">
        <td><code>atmega8</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega8515" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega8515 microchip avr dip-40 verified (p0-p5)">
        <td><code>atmega8515</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega8535" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega8535 microchip avr dip-40 verified (p0-p5)">
        <td><code>atmega8535</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega88" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega88 microchip avr dip-28 verified (p0-p5)">
        <td><code>atmega88</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atmega88p" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="atmega88p microchip avr dip-28 verified (p0-p5)">
        <td><code>atmega88p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny13" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny13 microchip avr dip-8 verified (p0-p5)">
        <td><code>attiny13</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-8</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny2313" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny2313 microchip avr dip-20 verified (p0-p5)">
        <td><code>attiny2313</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny25" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny25 microchip avr dip-8 verified (p0-p5)">
        <td><code>attiny25</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-8</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny261" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny261 microchip avr dip-20 verified (p0-p5)">
        <td><code>attiny261</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny44" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny44 microchip avr dip-14 verified (p0-p5)">
        <td><code>attiny44</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny45" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny45 microchip avr dip-8 verified (p0-p5)">
        <td><code>attiny45</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-8</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny461" data-family="AVR" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="attiny461 microchip avr dip-20 modelled (private)">
        <td><code>attiny461</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny48" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny48 microchip avr dip-28 verified (p0-p5)">
        <td><code>attiny48</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny84" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny84 microchip avr dip-14 verified (p0-p5)">
        <td><code>attiny84</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny85" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny85 microchip avr dip-8 verified (p0-p5)">
        <td><code>attiny85</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-8</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny861" data-family="AVR" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="attiny861 microchip avr dip-20 modelled (private)">
        <td><code>attiny861</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="attiny88" data-family="AVR" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="attiny88 microchip avr dip-28 verified (p0-p5)">
        <td><code>attiny88</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">DIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="atsamd21 microchip arm cortex-m  l1 modelled">
        <td><code>atsamd21</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21e15a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21e15a microchip arm cortex-m tqfp32/qfn32 l0 estate">
        <td><code>atsamd21e15a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP32/QFN32</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21e16a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21e16a microchip arm cortex-m tqfp32/qfn32 l0 estate">
        <td><code>atsamd21e16a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP32/QFN32</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21e17a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21e17a microchip arm cortex-m tqfp32/qfn32 l0 estate">
        <td><code>atsamd21e17a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP32/QFN32</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21e18a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21e18a microchip arm cortex-m tqfp32/qfn32 l0 estate">
        <td><code>atsamd21e18a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP32/QFN32</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21g15a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21g15a microchip arm cortex-m tqfp48/qfn48 l0 estate">
        <td><code>atsamd21g15a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP48/QFN48</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21g16a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21g16a microchip arm cortex-m tqfp48/qfn48 l0 estate">
        <td><code>atsamd21g16a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP48/QFN48</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21g17a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21g17a microchip arm cortex-m tqfp48/qfn48 l0 estate">
        <td><code>atsamd21g17a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP48/QFN48</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21g18a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="atsamd21g18a microchip arm cortex-m  l1 modelled">
        <td><code>atsamd21g18a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21j15a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21j15a microchip arm cortex-m tqfp64/qfn64 l0 estate">
        <td><code>atsamd21j15a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP64/QFN64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21j16a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21j16a microchip arm cortex-m tqfp64/qfn64 l0 estate">
        <td><code>atsamd21j16a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP64/QFN64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21j17a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21j17a microchip arm cortex-m tqfp64/qfn64 l0 estate">
        <td><code>atsamd21j17a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP64/QFN64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd21j18a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L0 estate" data-status="supported" data-search="atsamd21j18a microchip arm cortex-m tqfp64/qfn64 l0 estate">
        <td><code>atsamd21j18a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TQFP64/QFN64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd51j19a" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="modelled" data-status="supported" data-search="atsamd51j19a microchip arm cortex-m  modelled">
        <td><code>atsamd51j19a</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atsamd51j19a-pybadge" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="modelled" data-status="supported" data-search="atsamd51j19a-pybadge microchip arm cortex-m  modelled">
        <td><code>atsamd51j19a-pybadge</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="sam4s16c" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="sam4s16c microchip arm cortex-m  l1 modelled">
        <td><code>sam4s16c</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="same70q21b" data-family="Arm Cortex-M" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="same70q21b microchip arm cortex-m  l1 modelled">
        <td><code>same70q21b</code></td>
        <td>Microchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic12f1501" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12f1501 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12f1501</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12f1822" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12f1822 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12f1822</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12f1840" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12f1840 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12f1840</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12f609" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12f609 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12f609</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12f615" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12f615 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12f615</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12f629" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12f629 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12f629</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12f675" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12f675 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12f675</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12f683" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12f683 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12f683</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12f752" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12f752 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12f752</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12hv752" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12hv752 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12hv752</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12lf1501" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12lf1501 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12lf1501</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12lf1822" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12lf1822 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12lf1822</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic12lf1840" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic12lf1840 microchip pic16 pdip-8 modelled (private)">
        <td><code>pic12lf1840</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-8</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16c63a" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="excluded" data-search="pic16c63a microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16c63a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--muted" title="Microchip lists PIC16C63A as Not Recommended for new designs. Classic OTP, not buyable. Dropped with the PIC16C family in the parity tracker.">excluded</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16c65b" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="excluded" data-search="pic16c65b microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16c65b</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--muted" title="Microchip lists PIC16C65B as Not Recommended for new designs. Classic OTP, not buyable. Dropped with the PIC16C family in the parity tracker.">excluded</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16c73b" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="excluded" data-search="pic16c73b microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16c73b</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--muted" title="Microchip lists PIC16C73B as Not Recommended for new designs. Classic OTP, not buyable. Dropped with the PIC16C family in the parity tracker.">excluded</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16c74b" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="excluded" data-search="pic16c74b microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16c74b</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--muted" title="Microchip lists PIC16C74B as Not Recommended for new designs. Classic OTP, not buyable. Dropped with the PIC16C family in the parity tracker.">excluded</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1454" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1454 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16f1454</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1455" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1455 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16f1455</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1459" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1459 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f1459</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1503" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f1503 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16f1503</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1507" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f1507 microchip pic16 pdip-20 modelled (private)">
        <td><code>pic16f1507</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1508" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1508 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f1508</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1509" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1509 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f1509</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1516" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1516 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f1516</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1517" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1517 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1517</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1518" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1518 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f1518</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1519" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1519 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1519</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1613" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f1613 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16f1613</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1614" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1614 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16f1614</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1615" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1615 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16f1615</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1618" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1618 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f1618</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1619" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1619 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f1619</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1703" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f1703 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16f1703</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1704" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1704 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16f1704</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1705" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1705 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16f1705</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1707" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f1707 microchip pic16 pdip-20 modelled (private)">
        <td><code>pic16f1707</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1708" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1708 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f1708</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1709" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1709 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f1709</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1713" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1713 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16f1713</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1716" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1716 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16f1716</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1717" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1717 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1717</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1718" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1718 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f1718</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1719" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1719 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1719</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1784" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1784 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1784</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1786" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1786 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f1786</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1787" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1787 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1787</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1788" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1788 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f1788</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1789" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1789 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1789</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1823" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1823 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16f1823</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1824" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1824 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16f1824</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1825" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1825 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16f1825</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1826" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1826 microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16f1826</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1827" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1827 microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16f1827</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1828" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1828 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f1828</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1829" data-family="PIC16" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic16f1829 microchip pic16  l1 modelled">
        <td><code>pic16f1829</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1847" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1847 microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16f1847</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1933" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1933 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f1933</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1934" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1934 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1934</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1936" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1936 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f1936</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1937" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1937 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1937</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1938" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1938 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f1938</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1939" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1939 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f1939</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1946" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1946 microchip pic16 tqfp-64 verified (p0-p5)">
        <td><code>pic16f1946</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f1947" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f1947 microchip pic16 tqfp-64 verified (p0-p5)">
        <td><code>pic16f1947</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f610" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f610 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16f610</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f616" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f616 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16f616</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f627a" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f627a microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16f627a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f628a" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f628a microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16f628a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f630" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f630 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16f630</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f631" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f631 microchip pic16 pdip-20 modelled (private)">
        <td><code>pic16f631</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f648a" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f648a microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16f648a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f676" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f676 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16f676</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f677" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f677 microchip pic16 pdip-20 modelled (private)">
        <td><code>pic16f677</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f684" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f684 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16f684</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f685" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f685 microchip pic16 pdip-20 modelled (private)">
        <td><code>pic16f685</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f687" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f687 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f687</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f689" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f689 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f689</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f690" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f690 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16f690</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f707" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f707 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f707</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f716" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f716 microchip pic16 pdip-18 modelled (private)">
        <td><code>pic16f716</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f722a" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f722a microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f722a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f723a" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f723a microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16f723a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f724" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f724 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f724</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f726" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f726 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16f726</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f727" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f727 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f727</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f785" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f785 microchip pic16 pdip-20 modelled (private)">
        <td><code>pic16f785</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f818" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f818 microchip pic16 pdip-18 modelled (private)">
        <td><code>pic16f818</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f819" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f819 microchip pic16 pdip-18 modelled (private)">
        <td><code>pic16f819</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f84a" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16f84a microchip pic16 pdip-18 modelled (private)">
        <td><code>pic16f84a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f87" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f87 microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16f87</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f877a" data-family="PIC16" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic16f877a microchip pic16  l1 modelled">
        <td><code>pic16f877a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f882" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f882 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16f882</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f884" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f884 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f884</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f886" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f886 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16f886</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f913" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f913 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16f913</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f914" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f914 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f914</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f916" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f916 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16f916</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f917" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f917 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16f917</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f946" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16f946 microchip pic16 tqfp-64 verified (p0-p5)">
        <td><code>pic16f946</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16hv616" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16hv616 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16hv616</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1454" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1454 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16lf1454</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1455" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1455 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16lf1455</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1459" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1459 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16lf1459</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1503" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16lf1503 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16lf1503</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1507" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16lf1507 microchip pic16 pdip-20 modelled (private)">
        <td><code>pic16lf1507</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1508" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1508 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16lf1508</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1509" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1509 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16lf1509</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1516" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1516 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf1516</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1517" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1517 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1517</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1518" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1518 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf1518</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1519" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1519 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1519</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1613" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16lf1613 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16lf1613</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1614" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1614 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16lf1614</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1615" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1615 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16lf1615</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1618" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1618 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16lf1618</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1619" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1619 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16lf1619</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1703" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16lf1703 microchip pic16 pdip-14 modelled (private)">
        <td><code>pic16lf1703</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1704" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1704 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16lf1704</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1705" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1705 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16lf1705</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1707" data-family="PIC16" data-vendor="Microchip" data-coverage="modelled (private)" data-status="supported" data-search="pic16lf1707 microchip pic16 pdip-20 modelled (private)">
        <td><code>pic16lf1707</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1708" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1708 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16lf1708</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1709" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1709 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16lf1709</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1713" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1713 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16lf1713</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1716" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1716 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16lf1716</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1717" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1717 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1717</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1718" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1718 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf1718</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1719" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1719 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1719</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1784" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1784 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1784</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1786" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1786 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf1786</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1787" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1787 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1787</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1788" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1788 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf1788</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1789" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1789 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1789</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1823" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1823 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16lf1823</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1824" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1824 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16lf1824</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1825" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1825 microchip pic16 pdip-14 verified (p0-p5)">
        <td><code>pic16lf1825</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-14</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1826" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1826 microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16lf1826</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1827" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1827 microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16lf1827</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1828" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1828 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16lf1828</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1829" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1829 microchip pic16 pdip-20 verified (p0-p5)">
        <td><code>pic16lf1829</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1847" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1847 microchip pic16 pdip-18 verified (p0-p5)">
        <td><code>pic16lf1847</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-18</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1933" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1933 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf1933</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1934" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1934 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1934</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1936" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1936 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf1936</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1937" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1937 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1937</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1938" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1938 microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf1938</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1939" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1939 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf1939</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1946" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1946 microchip pic16 tqfp-64 verified (p0-p5)">
        <td><code>pic16lf1946</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf1947" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf1947 microchip pic16 tqfp-64 verified (p0-p5)">
        <td><code>pic16lf1947</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf707" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf707 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf707</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf722a" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf722a microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf722a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf723a" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf723a microchip pic16 spdip-28 verified (p0-p5)">
        <td><code>pic16lf723a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf724" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf724 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf724</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf726" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf726 microchip pic16 pdip-28 verified (p0-p5)">
        <td><code>pic16lf726</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic16lf727" data-family="PIC16" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic16lf727 microchip pic16 pdip-40 verified (p0-p5)">
        <td><code>pic16lf727</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f13k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f13k22 microchip pic18 pdip-20 verified (p0-p5)">
        <td><code>pic18f13k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f14k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f14k22 microchip pic18 pdip-20 verified (p0-p5)">
        <td><code>pic18f14k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2220" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2220 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2220</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2221" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2221 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2221</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2320" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2320 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2320</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2321" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2321 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2321</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2331" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2331 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2331</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f23k20" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f23k20 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f23k20</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f23k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f23k22 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f23k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2410" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2410 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2410</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f242" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f242 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f242</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2420" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2420 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2420</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2423" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2423 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2423</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2431" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2431 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2431</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2450" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2450 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2450</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2455" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2455 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2455</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2458" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2458 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2458</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f248" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f248 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f248</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2480" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2480 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2480</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f24j10" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f24j10 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f24j10</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f24k20" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f24k20 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f24k20</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f24k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f24k22 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f24k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f24k50" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f24k50 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f24k50</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2510" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2510 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2510</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2515" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2515 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2515</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f252" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f252 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f252</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2520" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2520 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2520</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2523" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2523 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2523</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2525" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2525 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2525</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2550" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2550 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2550</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2553" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2553 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2553</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f258" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f258 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f258</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2580" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2580 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2580</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2585" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2585 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2585</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f25j10" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f25j10 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f25j10</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f25k20" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f25k20 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f25k20</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f25k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f25k22 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f25k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f25k50" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f25k50 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f25k50</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f25k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f25k80 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f25k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2610" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2610 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2610</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2620" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2620 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2620</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2680" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2680 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2680</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2682" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2682 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2682</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f2685" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f2685 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f2685</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f26k20" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f26k20 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f26k20</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f26k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f26k22 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f26k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f26k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f26k80 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18f26k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4220" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4220 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4220</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4221" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4221 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4221</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4320" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4320 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4320</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4321" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4321 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4321</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4331" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4331 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4331</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f43k20" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f43k20 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f43k20</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f43k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f43k22 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f43k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4410" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4410 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4410</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f442" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f442 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f442</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4420" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4420 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4420</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4423" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4423 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4423</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4431" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4431 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4431</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4450" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4450 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4450</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4455" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4455 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4455</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4458" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4458 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4458</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f448" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f448 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f448</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4480" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4480 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4480</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f44j10" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f44j10 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f44j10</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f44k20" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f44k20 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f44k20</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f44k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f44k22 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f44k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4510" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4510 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4510</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4515" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4515 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4515</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f452" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f452 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f452</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4520" data-family="PIC18" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic18f4520 microchip pic18  l1 modelled">
        <td><code>pic18f4520</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4523" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4523 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4523</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4525" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4525 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4525</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4550" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4550 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4550</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4553" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4553 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4553</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f458" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f458 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f458</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4580" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4580 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4580</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4585" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4585 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4585</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f45j10" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f45j10 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f45j10</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f45k20" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f45k20 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f45k20</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f45k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f45k22 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f45k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f45k50" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f45k50 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f45k50</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f45k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f45k80 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f45k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4610" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4610 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4610</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4620" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4620 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4620</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4680" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4680 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4680</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4682" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4682 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4682</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4685" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f4685 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f4685</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f46k20" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f46k20 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f46k20</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f46k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f46k22 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f46k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f46k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f46k80 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18f46k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6390" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6390 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6390</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6393" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6393 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6393</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f63j90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f63j90 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f63j90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6490" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6490 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6490</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6493" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6493 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6493</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f64j90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f64j90 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f64j90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6520" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6520 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6520</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6527" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6527 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6527</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6585" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6585 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6585</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f65j90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f65j90 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f65j90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f65k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f65k22 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f65k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f65k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f65k80 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f65k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f65k90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f65k90 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f65k90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6620" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6620 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6620</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6622" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6622 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6622</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6627" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6627 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6627</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6628" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6628 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6628</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6680" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6680 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6680</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f66k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f66k22 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f66k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f66k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f66k80 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f66k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f66k90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f66k90 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f66k90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6720" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6720 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6720</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6722" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6722 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6722</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f6723" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f6723 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f6723</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f67k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f67k22 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f67k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f67k90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f67k90 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18f67k90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8390" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8390 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8390</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8393" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8393 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8393</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f83j90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f83j90 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f83j90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8490" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8490 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8490</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8493" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8493 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8493</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f84j90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f84j90 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f84j90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8520" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8520 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8520</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8527" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8527 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8527</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8585" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8585 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8585</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f85j90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f85j90 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f85j90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f85k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f85k22 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f85k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f85k90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f85k90 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f85k90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8620" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8620 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8620</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8622" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8622 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8622</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8627" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8627 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8627</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8680" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8680 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8680</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f86k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f86k22 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f86k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f86k90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f86k90 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f86k90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8720" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8720 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8720</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f8722" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f8722 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f8722</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f87k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f87k22 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f87k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f87k90" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18f87k90 microchip pic18 tqfp-80 verified (p0-p5)">
        <td><code>pic18f87k90</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-80</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf13k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf13k22 microchip pic18 pdip-20 verified (p0-p5)">
        <td><code>pic18lf13k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf14k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf14k22 microchip pic18 pdip-20 verified (p0-p5)">
        <td><code>pic18lf14k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-20</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf23k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf23k22 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18lf23k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf24k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf24k22 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18lf24k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf24k50" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf24k50 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18lf24k50</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf25k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf25k22 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18lf25k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf25k50" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf25k50 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18lf25k50</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf25k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf25k80 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18lf25k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf26k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf26k22 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18lf26k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf26k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf26k80 microchip pic18 spdip-28 verified (p0-p5)">
        <td><code>pic18lf26k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">SPDIP-28</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf43k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf43k22 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18lf43k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf44k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf44k22 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18lf44k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf45k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf45k22 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18lf45k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf45k50" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf45k50 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18lf45k50</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf45k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf45k80 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18lf45k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf46k22" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf46k22 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18lf46k22</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf46k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf46k80 microchip pic18 pdip-40 verified (p0-p5)">
        <td><code>pic18lf46k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">PDIP-40</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf65k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf65k80 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18lf65k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic18lf66k80" data-family="PIC18" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic18lf66k80 microchip pic18 tqfp-64 verified (p0-p5)">
        <td><code>pic18lf66k80</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">TQFP-64</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj12gp201" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="dspic33fj12gp201 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>dspic33fj12gp201</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj12gp202" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="dspic33fj12gp202 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>dspic33fj12gp202</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj12mc201" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="dspic33fj12mc201 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>dspic33fj12mc201</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj12mc202" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="dspic33fj12mc202 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>dspic33fj12mc202</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj16gp304" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="dspic33fj16gp304 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>dspic33fj16gp304</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj16mc304" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="dspic33fj16mc304 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>dspic33fj16mc304</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj32gp202" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="dspic33fj32gp202 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>dspic33fj32gp202</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj32gp204" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="dspic33fj32gp204 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>dspic33fj32gp204</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj32mc202" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="dspic33fj32mc202 microchip pic24 / dspic33  l1 modelled">
        <td><code>dspic33fj32mc202</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="dspic33fj32mc204" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="dspic33fj32mc204 microchip pic24 / dspic33  l1 modelled">
        <td><code>dspic33fj32mc204</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic24f04ka200" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic24f04ka200 microchip pic24 / dspic33  l1 modelled">
        <td><code>pic24f04ka200</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic24f04ka201" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic24f04ka201 microchip pic24 / dspic33  l1 modelled">
        <td><code>pic24f04ka201</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic24f08ka101" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic24f08ka101 microchip pic24 / dspic33  l1 modelled">
        <td><code>pic24f08ka101</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic24f08ka102" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic24f08ka102 microchip pic24 / dspic33  l1 modelled">
        <td><code>pic24f08ka102</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic24f16ka101" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic24f16ka101 microchip pic24 / dspic33  l1 modelled">
        <td><code>pic24f16ka101</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic24f16ka102" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic24f16ka102 microchip pic24 / dspic33  l1 modelled">
        <td><code>pic24f16ka102</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj128ga006" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj128ga006 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj128ga006</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj128ga008" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj128ga008 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj128ga008</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj128ga010" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj128ga010 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj128ga010</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj128ga106" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj128ga106 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj128ga106</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj128ga108" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj128ga108 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj128ga108</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj128ga110" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj128ga110 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj128ga110</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj16ga002" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj16ga002 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj16ga002</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj16ga004" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj16ga004 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj16ga004</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj192ga106" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj192ga106 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj192ga106</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj192ga108" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj192ga108 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj192ga108</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj192ga110" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj192ga110 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj192ga110</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj256ga106" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj256ga106 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj256ga106</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj256ga108" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj256ga108 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj256ga108</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj256ga110" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj256ga110 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj256ga110</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj32ga002" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj32ga002 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj32ga002</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj32ga004" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj32ga004 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj32ga004</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj32ga102" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj32ga102 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj32ga102</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj32ga104" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj32ga104 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj32ga104</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj48ga002" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj48ga002 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj48ga002</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj48ga004" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj48ga004 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj48ga004</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga002" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga002 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga002</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga004" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga004 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga004</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga006" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga006 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga006</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga008" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga008 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga008</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga010" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga010 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga010</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga102" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga102 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga102</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga104" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga104 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga104</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga106" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga106 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga106</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga108" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga108 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga108</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj64ga110" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj64ga110 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj64ga110</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj96ga006" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj96ga006 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj96ga006</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj96ga008" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj96ga008 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj96ga008</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="pic24fj96ga010" data-family="PIC24 / dsPIC33" data-vendor="Microchip" data-coverage="verified (P0-P5)" data-status="supported" data-search="pic24fj96ga010 microchip pic24 / dspic33  verified (p0-p5)">
        <td><code>pic24fj96ga010</code></td>
        <td>Microchip</td>
        <td>PIC24 / dsPIC33</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="imxrt1064" data-family="Arm Cortex-M" data-vendor="NXP" data-coverage="L1 modelled" data-status="supported" data-search="imxrt1064 nxp arm cortex-m  l1 modelled">
        <td><code>imxrt1064</code></td>
        <td>NXP</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="k32w041z" data-family="Arm Cortex-M" data-vendor="NXP" data-coverage="modelled (private)" data-status="supported" data-search="k32w041z nxp arm cortex-m  modelled (private)">
        <td><code>k32w041z</code></td>
        <td>NXP</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="mkw41z4" data-family="Arm Cortex-M" data-vendor="NXP" data-coverage="L1 modelled" data-status="supported" data-search="mkw41z4 nxp arm cortex-m  l1 modelled">
        <td><code>mkw41z4</code></td>
        <td>NXP</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="k32w041z-boot" data-family="Other" data-vendor="NXP" data-coverage="modelled (private)" data-status="supported" data-search="k32w041z-boot nxp other  modelled (private)">
        <td><code>k32w041z-boot</code></td>
        <td>NXP</td>
        <td>Other</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled (private)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="nrf51822" data-family="Arm Cortex-M" data-vendor="Nordic Semiconductor" data-coverage="L1 modelled" data-status="supported" data-search="nrf51822 nordic semiconductor arm cortex-m  l1 modelled">
        <td><code>nrf51822</code></td>
        <td>Nordic Semiconductor</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="nrf52832" data-family="Arm Cortex-M" data-vendor="Nordic Semiconductor" data-coverage="L1 modelled" data-status="supported" data-search="nrf52832 nordic semiconductor arm cortex-m  l1 modelled">
        <td><code>nrf52832</code></td>
        <td>Nordic Semiconductor</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span> <span class="lw-dim">T1 10✓</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="nrf52833" data-family="Arm Cortex-M" data-vendor="Nordic Semiconductor" data-coverage="L1 modelled" data-status="supported" data-search="nrf52833 nordic semiconductor arm cortex-m  l1 modelled">
        <td><code>nrf52833</code></td>
        <td>Nordic Semiconductor</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span> <span class="lw-dim">T1 12✓</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="nrf52840" data-family="Arm Cortex-M" data-vendor="Nordic Semiconductor" data-coverage="L1 modelled" data-status="supported" data-search="nrf52840 nordic semiconductor arm cortex-m  l1 modelled">
        <td><code>nrf52840</code></td>
        <td>Nordic Semiconductor</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span> <span class="lw-dim">T1 10✓</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="nrf5340" data-family="Arm Cortex-M" data-vendor="Nordic Semiconductor" data-coverage="L1 modelled" data-status="supported" data-search="nrf5340 nordic semiconductor arm cortex-m  l1 modelled">
        <td><code>nrf5340</code></td>
        <td>Nordic Semiconductor</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="nrf54l15" data-family="Arm Cortex-M" data-vendor="Nordic Semiconductor" data-coverage="L1 modelled" data-status="supported" data-search="nrf54l15 nordic semiconductor arm cortex-m  l1 modelled">
        <td><code>nrf54l15</code></td>
        <td>Nordic Semiconductor</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="nrf54lm20a" data-family="Arm Cortex-M" data-vendor="Nordic Semiconductor" data-coverage="L1 modelled" data-status="supported" data-search="nrf54lm20a nordic semiconductor arm cortex-m  l1 modelled">
        <td><code>nrf54lm20a</code></td>
        <td>Nordic Semiconductor</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="vegaboard-ri5cy" data-family="RISC-V" data-vendor="OpenHW" data-coverage="L1 modelled" data-status="supported" data-search="vegaboard-ri5cy openhw risc-v  l1 modelled">
        <td><code>vegaboard-ri5cy</code></td>
        <td>OpenHW</td>
        <td>RISC-V</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="amap42kp" data-family="Arm Cortex-M" data-vendor="Other" data-coverage="L1 modelled" data-status="supported" data-search="amap42kp other arm cortex-m  l1 modelled">
        <td><code>amap42kp</code></td>
        <td>Other</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="mimxrt595s" data-family="Arm Cortex-M" data-vendor="Other" data-coverage="L1 modelled" data-status="supported" data-search="mimxrt595s other arm cortex-m  l1 modelled">
        <td><code>mimxrt595s</code></td>
        <td>Other</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="mk64f12" data-family="Arm Cortex-M" data-vendor="Other" data-coverage="L1 modelled" data-status="supported" data-search="mk64f12 other arm cortex-m  l1 modelled">
        <td><code>mk64f12</code></td>
        <td>Other</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="r7fa4m1ab" data-family="Arm Cortex-M" data-vendor="Other" data-coverage="modelled" data-status="supported" data-search="r7fa4m1ab other arm cortex-m  modelled">
        <td><code>r7fa4m1ab</code></td>
        <td>Other</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="r7fa6m5bh" data-family="Arm Cortex-M" data-vendor="Other" data-coverage="L1 modelled" data-status="supported" data-search="r7fa6m5bh other arm cortex-m  l1 modelled">
        <td><code>r7fa6m5bh</code></td>
        <td>Other</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="r7fa8m1ah" data-family="Arm Cortex-M" data-vendor="Other" data-coverage="L1 modelled" data-status="supported" data-search="r7fa8m1ah other arm cortex-m  l1 modelled">
        <td><code>r7fa8m1ah</code></td>
        <td>Other</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="raspberrypipico" data-family="Arm Cortex-M" data-vendor="Other" data-coverage="verified (P0-P5)" data-status="supported" data-search="raspberrypipico other arm cortex-m raspberry pi pico module verified (p0-p5)">
        <td><code>raspberrypipico</code></td>
        <td>Other</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">Raspberry Pi Pico module</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="raspberrypipicow" data-family="Arm Cortex-M" data-vendor="Other" data-coverage="verified (P0-P5)" data-status="supported" data-search="raspberrypipicow other arm cortex-m raspberry pi pico module verified (p0-p5)">
        <td><code>raspberrypipicow</code></td>
        <td>Other</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">Raspberry Pi Pico module</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="s32k388" data-family="Arm Cortex-M" data-vendor="Other" data-coverage="L1 modelled" data-status="supported" data-search="s32k388 other arm cortex-m  l1 modelled">
        <td><code>s32k388</code></td>
        <td>Other</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="ci-fixture-cortex-m3-uart1" data-family="Other" data-vendor="Other" data-coverage="modelled" data-status="supported" data-search="ci-fixture-cortex-m3-uart1 other other  modelled">
        <td><code>ci-fixture-cortex-m3-uart1</code></td>
        <td>Other</td>
        <td>Other</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="ci-fixture-unknown-arch" data-family="Other" data-vendor="Other" data-coverage="modelled" data-status="supported" data-search="ci-fixture-unknown-arch other other  modelled">
        <td><code>ci-fixture-unknown-arch</code></td>
        <td>Other</td>
        <td>Other</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="ci-fixture-riscv" data-family="RISC-V" data-vendor="Other" data-coverage="modelled" data-status="supported" data-search="ci-fixture-riscv other risc-v  modelled">
        <td><code>ci-fixture-riscv</code></td>
        <td>Other</td>
        <td>RISC-V</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="arduinonanoesp32" data-family="Xtensa LX7" data-vendor="Other" data-coverage="verified (P0-P5)" data-status="supported" data-search="arduinonanoesp32 other xtensa lx7 arduino nano esp32 board verified (p0-p5)">
        <td><code>arduinonanoesp32</code></td>
        <td>Other</td>
        <td>Xtensa LX7</td>
        <td class="lw-dim">Arduino Nano ESP32 board</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="rp2040" data-family="Arm Cortex-M" data-vendor="Raspberry Pi" data-coverage="L1 modelled" data-status="supported" data-search="rp2040 raspberry pi arm cortex-m  l1 modelled">
        <td><code>rp2040</code></td>
        <td>Raspberry Pi</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span> <span class="lw-dim">T1 11✓</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="rp2350" data-family="Arm Cortex-M" data-vendor="Raspberry Pi" data-coverage="L1 modelled" data-status="supported" data-search="rp2350 raspberry pi arm cortex-m  l1 modelled">
        <td><code>rp2350</code></td>
        <td>Raspberry Pi</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="rv1103" data-family="Arm Cortex-M" data-vendor="Rockchip" data-coverage="modelled" data-status="supported" data-search="rv1103 rockchip arm cortex-m  modelled">
        <td><code>rv1103</code></td>
        <td>Rockchip</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="STM32G474RE" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32g474re stmicroelectronics arm cortex-m  modelled">
        <td><code>STM32G474RE</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="STM32WB55" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32wb55 stmicroelectronics arm cortex-m  modelled">
        <td><code>STM32WB55</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="STM32WBA52" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32wba52 stmicroelectronics arm cortex-m  modelled">
        <td><code>STM32WBA52</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f072" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L1 modelled" data-status="supported" data-search="stm32f072 stmicroelectronics arm cortex-m  l1 modelled">
        <td><code>stm32f072</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103c4" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f103c4 stmicroelectronics arm cortex-m lqfp48/ufqfpn48 l0 estate">
        <td><code>stm32f103c4</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP48/UFQFPN48</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103c6" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f103c6 stmicroelectronics arm cortex-m lqfp48/ufqfpn48 l0 estate">
        <td><code>stm32f103c6</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP48/UFQFPN48</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103c8" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32f103c8 stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32f103c8</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103c8t6" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="verified (P0-P5)" data-status="supported" data-search="stm32f103c8t6 stmicroelectronics arm cortex-m lqfp-48 verified (p0-p5)">
        <td><code>stm32f103c8t6</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP-48</td>
        <td><span class="lw-badge lw-badge--ok">verified (P0-P5)</span></td>
        <td class="lw-dim">private</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103r4" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f103r4 stmicroelectronics arm cortex-m lqfp64 l0 estate">
        <td><code>stm32f103r4</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103r4h" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f103r4h stmicroelectronics arm cortex-m tfbga64 l0 estate">
        <td><code>stm32f103r4h</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TFBGA64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103r6" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f103r6 stmicroelectronics arm cortex-m lqfp64 l0 estate">
        <td><code>stm32f103r6</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103r6h" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f103r6h stmicroelectronics arm cortex-m tfbga64 l0 estate">
        <td><code>stm32f103r6h</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">TFBGA64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103rb" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f103rb stmicroelectronics arm cortex-m lqfp64 l0 estate">
        <td><code>stm32f103rb</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103t4" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f103t4 stmicroelectronics arm cortex-m vfqfpn36 l0 estate">
        <td><code>stm32f103t4</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">VFQFPN36</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f103t6" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f103t6 stmicroelectronics arm cortex-m vfqfpn36 l0 estate">
        <td><code>stm32f103t6</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">VFQFPN36</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401cb" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401cb stmicroelectronics arm cortex-m ufqfpn48/wlcsp49 l0 estate">
        <td><code>stm32f401cb</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">UFQFPN48/WLCSP49</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401cc" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401cc stmicroelectronics arm cortex-m ufqfpn48/wlcsp49 l0 estate">
        <td><code>stm32f401cc</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">UFQFPN48/WLCSP49</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401cd" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401cd stmicroelectronics arm cortex-m ufqfpn48/wlcsp49 l0 estate">
        <td><code>stm32f401cd</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">UFQFPN48/WLCSP49</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401cdu6" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L1 modelled" data-status="supported" data-search="stm32f401cdu6 stmicroelectronics arm cortex-m  l1 modelled">
        <td><code>stm32f401cdu6</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401ce" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401ce stmicroelectronics arm cortex-m ufqfpn48/wlcsp49 l0 estate">
        <td><code>stm32f401ce</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">UFQFPN48/WLCSP49</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401rb" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401rb stmicroelectronics arm cortex-m lqfp64 l0 estate">
        <td><code>stm32f401rb</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401rc" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401rc stmicroelectronics arm cortex-m lqfp64 l0 estate">
        <td><code>stm32f401rc</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401rd" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401rd stmicroelectronics arm cortex-m lqfp64 l0 estate">
        <td><code>stm32f401rd</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401re" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401re stmicroelectronics arm cortex-m  l0 estate">
        <td><code>stm32f401re</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401re" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401re stmicroelectronics arm cortex-m lqfp64 l0 estate">
        <td><code>stm32f401re</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP64</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401vb" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401vb stmicroelectronics arm cortex-m lqfp100/ufbga100 l0 estate">
        <td><code>stm32f401vb</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP100/UFBGA100</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401vc" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401vc stmicroelectronics arm cortex-m lqfp100/ufbga100 l0 estate">
        <td><code>stm32f401vc</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP100/UFBGA100</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401vd" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401vd stmicroelectronics arm cortex-m lqfp100/ufbga100 l0 estate">
        <td><code>stm32f401vd</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP100/UFBGA100</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f401ve" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f401ve stmicroelectronics arm cortex-m lqfp100/ufbga100 l0 estate">
        <td><code>stm32f401ve</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP100/UFBGA100</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f405rg" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32f405rg stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32f405rg</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f407ie" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f407ie stmicroelectronics arm cortex-m lqfp176/ufbga176 l0 estate">
        <td><code>stm32f407ie</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP176/UFBGA176</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f407ig" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f407ig stmicroelectronics arm cortex-m lqfp176/ufbga176 l0 estate">
        <td><code>stm32f407ig</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP176/UFBGA176</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f407ve" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f407ve stmicroelectronics arm cortex-m lqfp100 l0 estate">
        <td><code>stm32f407ve</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP100</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f407vg" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f407vg stmicroelectronics arm cortex-m lqfp100 l0 estate">
        <td><code>stm32f407vg</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP100</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f407vgt6" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32f407vgt6 stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32f407vgt6</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f407ze" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f407ze stmicroelectronics arm cortex-m lqfp144 l0 estate">
        <td><code>stm32f407ze</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP144</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f407zg" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32f407zg stmicroelectronics arm cortex-m lqfp144 l0 estate">
        <td><code>stm32f407zg</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">LQFP144</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f411ceu6" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L1 modelled" data-status="supported" data-search="stm32f411ceu6 stmicroelectronics arm cortex-m  l1 modelled">
        <td><code>stm32f411ceu6</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f746" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L1 modelled" data-status="supported" data-search="stm32f746 stmicroelectronics arm cortex-m  l1 modelled">
        <td><code>stm32f746</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32f767zi" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32f767zi stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32f767zi</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32g071rb" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32g071rb stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32g071rb</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32g0b1re" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L0 estate" data-status="supported" data-search="stm32g0b1re stmicroelectronics arm cortex-m  l0 estate">
        <td><code>stm32g0b1re</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">L0 estate</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32h563zi" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32h563zi stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32h563zi</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32h735vg" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32h735vg stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32h735vg</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32l073rz" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32l073rz stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32l073rz</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32l476rg" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32l476rg stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32l476rg</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32l476rg" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32l476rg stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32l476rg</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32l552" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L1 modelled" data-status="supported" data-search="stm32l552 stmicroelectronics arm cortex-m  l1 modelled">
        <td><code>stm32l552</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32u545re" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32u545re stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32u545re</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32u575zi" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="modelled" data-status="supported" data-search="stm32u575zi stmicroelectronics arm cortex-m  modelled">
        <td><code>stm32u575zi</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="stm32u5a5zj" data-family="Arm Cortex-M" data-vendor="STMicroelectronics" data-coverage="L1 modelled" data-status="supported" data-search="stm32u5a5zj stmicroelectronics arm cortex-m  l1 modelled">
        <td><code>stm32u5a5zj</code></td>
        <td>STMicroelectronics</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="EFR32MG26" data-family="Arm Cortex-M" data-vendor="Silicon Labs" data-coverage="modelled" data-status="supported" data-search="efr32mg26 silicon labs arm cortex-m  modelled">
        <td><code>EFR32MG26</code></td>
        <td>Silicon Labs</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--warn">modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="cc2538sf53" data-family="Arm Cortex-M" data-vendor="Texas Instruments" data-coverage="L1 modelled" data-status="supported" data-search="cc2538sf53 texas instruments arm cortex-m  l1 modelled">
        <td><code>cc2538sf53</code></td>
        <td>Texas Instruments</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
    </tbody>
  </table>
  <p class="lw-chips__empty" id="lw-chip-empty" hidden>No chip matches that filter.</p>
</div>

## Counts by family and vendor

| Vendor | Chips |
|---|---|
| Digilent | 1 |
| Espressif | 6 |
| LiteX | 1 |
| Microchip | 434 |
| NXP | 4 |
| Nordic Semiconductor | 7 |
| OpenHW | 1 |
| Other | 13 |
| Raspberry Pi | 2 |
| Rockchip | 1 |
| STMicroelectronics | 51 |
| Silicon Labs | 1 |
| Texas Instruments | 1 |

| Family | Chips |
|---|---|
| AVR | 55 |
| Arm Cortex-M | 92 |
| Other | 3 |
| PIC16 | 155 |
| PIC18 | 158 |
| PIC24 / dsPIC33 | 49 |
| RISC-V | 6 |
| Xtensa LX6 | 1 |
| Xtensa LX7 | 4 |

## Related coverage pages

- [Chip conformance](chip-conformance.md) — the L0/L1/L2 register scoreboard.
- [Tier-1 matrix](tier1-scoreboard.md) — per-subsystem pass matrix.
- [Arduino scoreboard](arduino-scoreboard.md) and [Zephyr scoreboard](zephyr-scoreboard.md) — board-level runs.

