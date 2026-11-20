# STM32F103 BME280 oversampling descriptor parse failure

Archived the first attempt after adding declarative oversampling skip behavior.
The CLI and firmware builds completed, then the imported-system smoke failed
while parsing `configs/devices/bme280.yaml`: the `TEMP` and `HUM` flow mappings
were missing their closing braces. The exact run logs and generated metrics are
retained here before the target metrics are replaced by the corrected attempt.
