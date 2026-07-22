/*
 * LabWired Zephyr matrix L1 — prove k_msleep / kernel tick advances.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

int main(void)
{
	printk("LW_Z1_BOOT\n");
	while (1) {
		k_msleep(50);
		printk("LW_Z1_OK\n");
	}
	return 0;
}
