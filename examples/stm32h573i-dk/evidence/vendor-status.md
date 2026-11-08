# Vendor status check — STM32H573I-DK

Checked live on 2026-11-04. No archived or Wayback source was used.

- ST product page: <https://www.st.com/en/evaluation-tools/stm32h573i-dk.html>
  reports **Active** and “Product is in volume production.” It identifies the
  kit as based on STM32H573IIK3Q.
- ST eStore listing:
  <https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-discovery-kits/stm32h573i-dk.html>
  reports **Active**, **In Stock**, **$155.58 at quantity 2**, and an **Add to
  Cart** option.
- ST STM32H573II product page:
  <https://www.st.com/en/microcontrollers-microprocessors/stm32h573ii.html>
  lists STM32H573IIK3Q as **Active — Product is in volume production**.

The board passes Rule 0 as an active, orderable product. The smoke profile only
claims the boot path and user LED LD1; it does not model the kit's LCD, Wi-Fi,
USB, Ethernet, audio, SD, external flash, or TrustZone setup.
