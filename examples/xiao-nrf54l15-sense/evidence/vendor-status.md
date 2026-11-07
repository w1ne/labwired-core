# Live vendor status: Seeed Studio XIAO nRF54L15 Sense

Checked 2026-11-04 UTC for the Sense board, SKU 101991422:

- [Seeed Studio product listing](https://jp.seeedstudio.com/XIAO-nRF54L15-Sense-p-6494.html): the live listing showed **In stock** at **¥2,512**. This is the exact Sense SKU onboarded here.
- [Nordic nRF54L15 product catalog](https://www.nordicsemi.com/Products): the current catalog lists nRF54L15. The catalog does not expose an explicit lifecycle label; the board's live Seeed listing is the buyability evidence.
- [ST LSM6DS3TR-C product page](https://www.st.com/en/mems-and-sensors/lsm6ds3tr-c.html): ST labels the IMU **Active** and in volume production.
- [ST eStore LSM6DS3TR-C listing](https://estore.st.com/en/products/mems-and-sensors/inemo-inertial-modules/lsm6ds3tr-c.html): ST labels the device **Active** and showed **In Stock** with an Add to Cart option. The IMU is physically on the board but is not modeled by this profile.
- [Seeed board guide](https://wiki.seeedstudio.com/xiao_nrf54l15_sense_getting_started/): identifies the Sense-only onboard LSM6DS3TR-C IMU and MSM261DGT006 microphone. Neither is claimed by the system descriptor.

No Wayback snapshot was used. This round claims the board's GPIO user LED only; the onboard sensors and their interfaces remain outside the model.
