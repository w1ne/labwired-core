# Chip list

The public chip catalogue: **114 parts** with an open model in [labwired-core](https://github.com/w1ne/labwired-core). 0 retired part(s) are kept in the table marked excluded. Commercial (private) models are listed in the LabWired app under your account.

The table is generated from the live catalogues by `scripts/generate_chip_catalog.py` and refreshed in CI, so it cannot drift from the models.

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
      <option value="excluded">excluded only</option>
    </select></label>
  </div>
  <p class="lw-chips__count" id="lw-chip-count">Showing 114 of 114 chips.</p>
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
      <tr class="lw-chip" data-name="litex-vexriscv" data-family="RISC-V" data-vendor="LiteX" data-coverage="L1 modelled" data-status="supported" data-search="litex-vexriscv litex risc-v  l1 modelled">
        <td><code>litex-vexriscv</code></td>
        <td>LiteX</td>
        <td>RISC-V</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="atmega1281" data-family="AVR" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="atmega1281 microchip avr  l1 modelled">
        <td><code>atmega1281</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
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
      <tr class="lw-chip" data-name="atmega328p" data-family="AVR" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="atmega328p microchip avr  l1 modelled">
        <td><code>atmega328p</code></td>
        <td>Microchip</td>
        <td>AVR</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
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
      <tr class="lw-chip" data-name="pic16f1829" data-family="PIC16" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic16f1829 microchip pic16  l1 modelled">
        <td><code>pic16f1829</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic16f877a" data-family="PIC16" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic16f877a microchip pic16  l1 modelled">
        <td><code>pic16f877a</code></td>
        <td>Microchip</td>
        <td>PIC16</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="pic18f4520" data-family="PIC18" data-vendor="Microchip" data-coverage="L1 modelled" data-status="supported" data-search="pic18f4520 microchip pic18  l1 modelled">
        <td><code>pic18f4520</code></td>
        <td>Microchip</td>
        <td>PIC18</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
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
      <tr class="lw-chip" data-name="imxrt1064" data-family="Arm Cortex-M" data-vendor="NXP" data-coverage="L1 modelled" data-status="supported" data-search="imxrt1064 nxp arm cortex-m  l1 modelled">
        <td><code>imxrt1064</code></td>
        <td>NXP</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
      </tr>
      <tr class="lw-chip" data-name="mkw41z4" data-family="Arm Cortex-M" data-vendor="NXP" data-coverage="L1 modelled" data-status="supported" data-search="mkw41z4 nxp arm cortex-m  l1 modelled">
        <td><code>mkw41z4</code></td>
        <td>NXP</td>
        <td>Arm Cortex-M</td>
        <td class="lw-dim">—</td>
        <td><span class="lw-badge lw-badge--ok">L1 modelled</span></td>
        <td class="lw-dim">public</td>
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
| Espressif | 5 |
| LiteX | 1 |
| Microchip | 32 |
| NXP | 2 |
| Nordic Semiconductor | 7 |
| OpenHW | 1 |
| Other | 10 |
| Raspberry Pi | 2 |
| Rockchip | 1 |
| STMicroelectronics | 50 |
| Silicon Labs | 1 |
| Texas Instruments | 1 |

| Family | Chips |
|---|---|
| AVR | 4 |
| Arm Cortex-M | 88 |
| Other | 2 |
| PIC16 | 2 |
| PIC18 | 1 |
| PIC24 / dsPIC33 | 8 |
| RISC-V | 6 |
| Xtensa LX6 | 1 |
| Xtensa LX7 | 2 |

## Related coverage pages

- [Chip conformance](chip-conformance.md) — the L0/L1/L2 register scoreboard.
- [Tier-1 matrix](tier1-scoreboard.md) — per-subsystem pass matrix.
- [Arduino scoreboard](arduino-scoreboard.md) and [Zephyr scoreboard](zephyr-scoreboard.md) — board-level runs.

