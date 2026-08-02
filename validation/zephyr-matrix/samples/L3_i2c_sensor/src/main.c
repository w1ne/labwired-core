/*
 * LabWired Zephyr matrix L3 — I2C probe of INA219 @ 0x40
 * (system external_devices). Stock Zephyr i2c driver only.
 *
 * Device selection prefers common aliases / labels so one sample
 * builds across Nucleo, nRF DK, and Pico without board overlays.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/devicetree.h>

#ifndef INA219_ADDR
#define INA219_ADDR 0x40
#endif

/* Prefer arduino_i2c / well-known labels (board-specific). */
#if DT_NODE_HAS_STATUS(DT_NODELABEL(arduino_i2c), okay)
#define I2C_NODE DT_NODELABEL(arduino_i2c)
#elif DT_NODE_HAS_STATUS(DT_NODELABEL(i2c1), okay)
#define I2C_NODE DT_NODELABEL(i2c1)
#elif DT_NODE_HAS_STATUS(DT_NODELABEL(i2c0), okay)
#define I2C_NODE DT_NODELABEL(i2c0)
#elif DT_NODE_HAS_STATUS(DT_ALIAS(i2c_0), okay)
#define I2C_NODE DT_ALIAS(i2c_0)
#elif DT_NODE_HAS_STATUS(DT_ALIAS(i2c_1), okay)
#define I2C_NODE DT_ALIAS(i2c_1)
#else
#define I2C_NODE DT_INVALID_NODE
#endif

int main(void)
{
	printk("LW_Z3_BOOT\n");

#if !DT_NODE_EXISTS(I2C_NODE)
	printk("LW_Z3_FAIL no_i2c_dt\n");
	return 0;
#else
	const struct device *i2c = DEVICE_DT_GET(I2C_NODE);

	if (!device_is_ready(i2c)) {
		printk("LW_Z3_FAIL not_ready\n");
		return 0;
	}

	/* 1-byte write of reg pointer 0 = config reg: proves START/ADDR/ACK/data
	 * on F1 poll path (len=0 hangs waiting BTF on STM32F1 Zephyr driver). */
	uint8_t reg = 0x00;
	int ret = i2c_write(i2c, &reg, 1, INA219_ADDR);
	if (ret == 0) {
		printk("LW_Z3_OK\n");
		return 0;
	}
	printk("LW_Z3_FAIL err=%d\n", ret);
	return 0;
#endif
}
