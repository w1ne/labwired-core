# External Components — SK-AM62

None. The Zephyr images use the SoC's own MAIN_UART0 console and the A53
generic timer; the kit's FTDI USB-UART bridge, LEDs, user button, I2C GPIO
expanders, Ethernet PHYs and expansion headers are not modelled or attached.
When a later slice attaches one, it goes here with its fidelity tier and
descriptor path.

`analog-ntc-diode-system.yaml` adds two generic analog parts, solved by the
in-core MNA engine (not kit hardware):

| Part | Where | Model |
| :--- | :--- | :--- |
| 10 k NTC (beta 3950) with a 10 k pull-up to 3.3 V | Expansion connector pin 15, `EXP_GPIO0_32` (ball L23) = MAIN_GPIO0 GPIO 32 | netlist `R ... beta= tnom= temp=` |
| Diode with a 1 k series resistor from 3.3 V | Expansion connector pin 13, `EXP_GPIO0_42` (ball L21) = MAIN_GPIO0 GPIO 42 | netlist `D` + `.model D(IS N RS ...)` |
