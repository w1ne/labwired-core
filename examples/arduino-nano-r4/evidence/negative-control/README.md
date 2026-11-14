# Negative control

Same ELF and smoke assertions, but `configs/chips/ra4m1.yaml` restored to the
read-zero `usbfs` stub (the #1723 head, 52c3e146c). Result: FAIL 0/3 at the
5,000,000-step limit; PC stays at 0x7ff0, the `dcd_init` poll on
`SYSCFG.SCKE`. The pass in `../run/` comes from the USBFS register file.
