# External Components — PolarFire SoC Discovery Kit

None. The Zephyr images use the MSS's own CLINT and MMUART1 console; the kit's
USB-UART bridge, LEDs, buttons, DDR4 and the FPGA fabric are not modelled or
attached. When a later slice attaches one, it goes here with its fidelity tier
and descriptor path.

`analog-ntc-diode-system.yaml` adds two generic analog parts, solved by the
in-core MNA engine (not kit hardware):

| Part | Where | Model |
| :--- | :--- | :--- |
| 10 k NTC (beta 3950) with a 10 k pull-up to 3.3 V | RPI_GPIO4 (ball E12) = MSS GPIO_2 port 0 | netlist `R ... beta= tnom= temp=` |
| Diode with a 1 k series resistor from 3.3 V | RPI_GPIO17 (ball G18) = MSS GPIO_2 port 1 | netlist `D` + `.model D(IS N RS ...)` |
