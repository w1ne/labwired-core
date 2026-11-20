# Failed H753 ADC fixture schema attempt

The first target-only run stopped before booting. The smoke put the external
potentiometer fixture in the smoke script, but `external_devices` belongs in
the system descriptor. The smoke parser rejected the unknown field. The
original run logs, metrics, and simulator output are preserved in this
directory.
