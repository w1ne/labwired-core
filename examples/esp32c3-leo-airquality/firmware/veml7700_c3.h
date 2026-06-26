/* Compact ESP32-C3 driver for the Vishay VEML7700 ambient-light sensor.
 *
 * Unlike the three Sensirion parts on this board, the VEML7700 has no official
 * bare-metal vendor C driver (Vishay ships an Arduino C++ library), so this is
 * a small register-level driver written against the datasheet. It drives the
 * same simulated C3 I²C0 command-list engine as the Sensirion HAL.
 */
#ifndef VEML7700_C3_H
#define VEML7700_C3_H

#include <stdint.h>

#define VEML7700_I2C_ADDR 0x10
#define VEML7700_REG_ALS_CONF 0x00
#define VEML7700_REG_ALS 0x04

/* Bring the sensor out of shutdown (ALS_CONF = 0x0000: gain ×1, IT 100 ms). */
void veml7700_init(void);

/* Read the raw 16-bit ALS count. Returns 0 on success, -1 on I²C error. */
int veml7700_read_als(uint16_t *out_counts);

/* Convert a raw ALS count to lux at the default gain ×1 / IT 100 ms. */
uint32_t veml7700_counts_to_lux(uint16_t counts);

#endif /* VEML7700_C3_H */
