/* Melexis MLX90614 IR-thermometer driver for the ESP32-C3 sim.
 *
 * Issues an SMBus "read word" (write RAM command, repeated START, read LSB,
 * MSB, PEC) over the simulated C3 I²C0 controller and validates the CRC-8 PEC.
 * Used by the Leo board to read a cold surface temperature for dew-point /
 * condensation (mold) detection. */
#ifndef MLX90614_C3_H
#define MLX90614_C3_H

#include <stdint.h>

#define MLX90614_I2C_ADDR 0x5A
#define MLX90614_RAM_TA 0x06    /* ambient (chip) temperature */
#define MLX90614_RAM_TOBJ1 0x07 /* object 1 (surface) temperature */

/* Read a RAM temperature register. On success writes the temperature in
 * centi-degrees-Celsius (°C × 100) to *out_centi_c and returns 0. Returns -1 on
 * bus error and -2 on PEC (checksum) mismatch. */
int mlx90614_read_centi_c(uint8_t ram_cmd, int32_t *out_centi_c);

/* Convenience: read the object/surface temperature (TOBJ1). */
int mlx90614_read_surface_centi_c(int32_t *out_centi_c);

#endif /* MLX90614_C3_H */
