/*
 * LabWired Zephyr matrix L2 — GPIO LED0 + serial marker.
 * If the board has no led0 alias, still emit the marker (GPIO path skipped).
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/drivers/gpio.h>

#define SLEEP_MS 20

int main(void)
{
#if DT_NODE_HAS_STATUS(DT_ALIAS(led0), okay)
	const struct gpio_dt_spec led = GPIO_DT_SPEC_GET(DT_ALIAS(led0), gpios);
	bool have_led = false;
#else
	bool have_led = false;
#endif

	printk("LW_Z2_BOOT\n");

#if DT_NODE_HAS_STATUS(DT_ALIAS(led0), okay)
	if (gpio_is_ready_dt(&led) &&
	    gpio_pin_configure_dt(&led, GPIO_OUTPUT_ACTIVE) == 0) {
		have_led = true;
	}
#endif

	while (1) {
#if DT_NODE_HAS_STATUS(DT_ALIAS(led0), okay)
		if (have_led) {
			(void)gpio_pin_toggle_dt(&led);
		}
#else
		ARG_UNUSED(have_led);
#endif
		k_msleep(SLEEP_MS);
		printk("LW_Z2_OK\n");
	}
	return 0;
}
