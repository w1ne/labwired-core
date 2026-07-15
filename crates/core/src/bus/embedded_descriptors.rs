// Auto-generated: embeds esp32c3/rp2040 declarative peripheral descriptors into the
// binary so SystemBus::from_config can resolve them WITHOUT a filesystem
// (wasm32 has no std::fs). Native builds still fall back to from_file for any
// path not embedded here. Keyed by the suffix after 'peripherals/'.
//
// To refresh: re-run the generator over configs/peripherals/esp32c3/.

/// Look up an embedded descriptor by its chip-YAML `path:` value
/// (e.g. `../peripherals/esp32c3/gpio.yaml`). Returns the YAML text if embedded.
pub fn lookup(descriptor_path: &str) -> Option<&'static str> {
    // Normalize: take everything after the last "peripherals/".
    let key = descriptor_path
        .rsplit_once("peripherals/")
        .map(|(_, k)| k)
        .unwrap_or(descriptor_path);
    match key {
        "esp32c3/aes.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/aes.yaml"
        )),
        "esp32c3/apb_ctrl.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/apb_ctrl.yaml"
        )),
        "esp32c3/apb_saradc.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/apb_saradc.yaml"
        )),
        "esp32c3/assist_debug.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/assist_debug.yaml"
        )),
        "esp32c3/bb.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/bb.yaml"
        )),
        "esp32c3/dma.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/dma.yaml"
        )),
        "esp32c3/ds.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/ds.yaml"
        )),
        "esp32c3/efuse.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/efuse.yaml"
        )),
        "esp32c3/extmem.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/extmem.yaml"
        )),
        "esp32c3/gpio_sd.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/gpio_sd.yaml"
        )),
        "esp32c3/gpio.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/gpio.yaml"
        )),
        "esp32c3/hmac.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/hmac.yaml"
        )),
        "esp32c3/i2c0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/i2c0.yaml"
        )),
        "esp32c3/i2s0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/i2s0.yaml"
        )),
        "esp32c3/interrupt_core0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/interrupt_core0.yaml"
        )),
        "esp32c3/io_mux.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/io_mux.yaml"
        )),
        "esp32c3/ledc.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/ledc.yaml"
        )),
        "esp32c3/radio_fe.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/radio_fe.yaml"
        )),
        "esp32c3/radio_nrx.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/radio_nrx.yaml"
        )),
        "esp32c3/rmt.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/rmt.yaml"
        )),
        "esp32c3/rng.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/rng.yaml"
        )),
        "esp32c3/rom.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/rom.yaml"
        )),
        "esp32c3/rsa.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/rsa.yaml"
        )),
        "esp32c3/rtc_cntl.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/rtc_cntl.yaml"
        )),
        "esp32c3/sensitive.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/sensitive.yaml"
        )),
        "esp32c3/sha.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/sha.yaml"
        )),
        "esp32c3/spi0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/spi0.yaml"
        )),
        "esp32c3/spi1.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/spi1.yaml"
        )),
        "esp32c3/spi2.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/spi2.yaml"
        )),
        "esp32c3/system.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/system.yaml"
        )),
        "esp32c3/systimer.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/systimer.yaml"
        )),
        "esp32c3/timg0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/timg0.yaml"
        )),
        "esp32c3/timg1.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/timg1.yaml"
        )),
        "esp32c3/twai0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/twai0.yaml"
        )),
        "esp32c3/uart0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/uart0.yaml"
        )),
        "esp32c3/uart1.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/uart1.yaml"
        )),
        "esp32c3/uhci0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/uhci0.yaml"
        )),
        "esp32c3/uhci1.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/uhci1.yaml"
        )),
        "esp32c3/usb_device.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/usb_device.yaml"
        )),
        "esp32c3/wifi_mac.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/wifi_mac.yaml"
        )),
        "esp32c3/xts_aes.yaml" => Some(include_str!(
            "../../../../configs/peripherals/esp32c3/xts_aes.yaml"
        )),
        "rp2040/sysinfo.yaml" => Some(include_str!(
            "../../../../configs/peripherals/rp2040/sysinfo.yaml"
        )),
        // NXP KW41Z (mkw41z4) declarative descriptors — embedded so the chip's
        // declarative peripherals load in the wasm playground (no std::fs).
        "mkw41z4/gpioa.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/gpioa.yaml"
        )),
        "mkw41z4/gpiob.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/gpiob.yaml"
        )),
        "mkw41z4/gpioc.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/gpioc.yaml"
        )),
        "mkw41z4/i2c0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/i2c0.yaml"
        )),
        "mkw41z4/lpuart0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/lpuart0.yaml"
        )),
        "mkw41z4/mcg.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/mcg.yaml"
        )),
        "mkw41z4/pit.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/pit.yaml"
        )),
        "mkw41z4/pmc.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/pmc.yaml"
        )),
        "mkw41z4/porta.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/porta.yaml"
        )),
        "mkw41z4/portb.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/portb.yaml"
        )),
        "mkw41z4/portc.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/portc.yaml"
        )),
        "mkw41z4/rcm.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/rcm.yaml"
        )),
        "mkw41z4/sim.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/sim.yaml"
        )),
        "mkw41z4/smc.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/smc.yaml"
        )),
        "mkw41z4/spi0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/spi0.yaml"
        )),
        "mkw41z4/tpm0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/tpm0.yaml"
        )),
        "mkw41z4/trng0.yaml" => Some(include_str!(
            "../../../../configs/peripherals/mkw41z4/trng0.yaml"
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::lookup;

    #[test]
    fn rp2040_sysinfo_descriptor_is_embedded_for_wasm_browser_boot() {
        assert!(lookup("../peripherals/rp2040/sysinfo.yaml").is_some());
        assert!(lookup("rp2040/sysinfo.yaml").is_some());
    }
}
