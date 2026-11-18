# PCA9685 TWI firmware fixture

`pca_servo_twi.elf` was built from the adjacent C source using avr-gcc (GCC)
7.3.0 with:

```sh
avr-gcc -mmcu=atmega328p -Os -nostdlib -Wl,-e,main \
  tests/fixtures/avr/pca_servo_twi.c -o tests/fixtures/avr/pca_servo_twi.elf
```

The fixture sends a PCA9685 command followed by FULL_OFF through TWDR/TWCR. It
is an instruction-level simulator fixture, not evidence of physical operation.
The generated ELF SHA-256 is
`5ece68405fb24188dddcd6a264e764be5bf728dbc2738e07176bb9ff51eee01c`.
