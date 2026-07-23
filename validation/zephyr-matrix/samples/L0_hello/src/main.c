/*
 * LabWired Zephyr matrix L0 — prove kernel + console after boot.
 * Marker must appear once; loop may idle.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

int main(void)
{
	printk("LW_Z0_OK\n");
	while (1) {
		k_sleep(K_FOREVER);
	}
	return 0;
}
