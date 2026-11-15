//! Actual ESP-IDF customer startup prerequisite; no electrical IO-Link claim.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32c3_rom::{
    build_rom_boot_machine, inject_rom_regions, provision_rom_images, RomBootOpts,
};
use labwired_core::{bus::SystemBus, Bus, Cpu};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "requires the ESP-IDF customer ELF and sibling flash images in IOLINKI_C3_ELF"]
fn customer_c3_esp_idf_firmware_initializes_its_gpio_and_uart_driver() {
    let elf = std::env::var("IOLINKI_C3_ELF").expect("IOLINKI_C3_ELF required");
    let bytes = std::fs::read(&elf).unwrap();
    let symbol = |name| labwired_loader::resolve_symbol_in_elf(&bytes, name).expect(name);
    let installed = symbol("installed");
    let events = symbol("uart_events");
    let guard = symbol("tx_guard");
    let tick = symbol("reference_device_tick");
    let app_main = symbol("app_main");
    let app_desc = symbol("esp_app_desc") as u64;
    let path = Path::new(&elf).parent().unwrap();
    // Execute the second-stage image through the real flash path, and require
    // every app-image load segment to contain the exact supplied ELF bytes.
    // esptool omits ELF dummy alignment sections and pads segment tails.
    let program = labwired_loader::load_elf(Path::new(&elf)).unwrap();
    let app_image = std::fs::read(path.join("firmware.bin")).unwrap();
    assert_eq!(app_image[0], 0xe9);
    assert_eq!(
        u32::from_le_bytes(app_image[4..8].try_into().unwrap()) as u64,
        program.entry_point
    );
    let mut at = 24;
    for _ in 0..app_image[1] {
        let address = u32::from_le_bytes(app_image[at..at + 4].try_into().unwrap()) as u64;
        let len = u32::from_le_bytes(app_image[at + 4..at + 8].try_into().unwrap()) as usize;
        at += 8;
        let segment = program
            .segments
            .iter()
            .find(|s| address >= s.start_addr && address < s.start_addr + s.data.len() as u64)
            .expect("image segment covered by ELF");
        let offset = (address - segment.start_addr) as usize;
        let copied = len.min(segment.data.len() - offset);
        let mut expected = segment.data[offset..offset + copied].to_vec();
        // esptool fills esp_app_desc.elf_sha256 in the binary after linking.
        if address <= app_desc && app_desc + 176 <= address + copied as u64 {
            let digest_at = (app_desc + 144 - address) as usize;
            expected[digest_at..digest_at + 32].copy_from_slice(&Sha256::digest(&bytes));
        }
        assert!(
            app_image[at..at + copied] == expected,
            "actual ELF bytes at {address:#x} differ"
        );
        assert!(
            app_image[at + copied..at + len].iter().all(|&b| b == 0),
            "only zero image tail padding permitted"
        );
        at += len;
    }
    let mut flash = vec![0xff; 4 * 1024 * 1024];
    for (name, offset) in [
        ("bootloader.bin", 0),
        ("partitions.bin", 0x8000),
        ("firmware.bin", 0x10000),
    ] {
        let image = std::fs::read(path.join(name)).expect(name);
        flash[offset..offset + image.len()].copy_from_slice(&image);
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/esp32c3.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        "name: c3-reference\nchip: esp32c3.yaml\nexternal_devices: []\nboard_io: []\n",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    assert!(inject_rom_regions(
        &mut bus,
        &provision_rom_images().expect("genuine C3 mask ROM")
    ));
    let gpio = bus.find_peripheral_index_by_name("gpio").unwrap();
    for pin in [0, 5, 7, 10] {
        assert!(bus.set_peripheral_gpio_input(gpio, pin, true));
    }
    let console = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(console.clone(), false);
    let mut machine = build_rom_boot_machine(bus, flash, RomBootOpts::default(), |cpu| cpu);
    let mut entered_app = false;
    let mut sampled = false;
    for _ in 0..30_000_000 {
        entered_app |= machine.cpu.get_pc() == app_main;
        sampled |= machine.cpu.get_pc() == tick;
        if sampled {
            break;
        }
        if let Err(error) = machine.step() {
            panic!(
                "real ESP-IDF boot failed at PC={:#x}: {error}; console={}",
                machine.cpu.get_pc(),
                String::from_utf8_lossy(&console.lock().unwrap())
            );
        }
    }
    assert!(
        entered_app && sampled,
        "actual app_main and reference sample must execute; PC={:#x}; console={}",
        machine.cpu.get_pc(),
        String::from_utf8_lossy(&console.lock().unwrap())
    );
    assert_eq!(
        machine.bus.read_u8(installed as u64).unwrap(),
        1,
        "ESP-IDF UART driver initialized"
    );
    assert_ne!(
        machine.bus.read_u32(events as u64).unwrap(),
        0,
        "SDK UART event queue allocated"
    );
    assert_eq!(
        machine.bus.read_u8(guard as u64).unwrap(),
        0,
        "TX guardian has no latched fault"
    );
    assert_eq!(
        machine.bus.read_u32(0x60004020).unwrap() & 0x52,
        0x52,
        "EN, TX, LED configured as outputs"
    );
    assert_eq!(
        machine.bus.read_u32(0x60004004).unwrap() & (1 << 6),
        1 << 6,
        "L6362A enabled for SIO"
    );
    assert_eq!(
        machine.bus.read_u32(0x60004564).unwrap() & 0x1ff,
        128,
        "GPIO4 SIO uses GPIO output matrix"
    );
    machine.bus.set_peripheral_gpio_input(gpio, 7, false);
    assert_eq!(
        machine.bus.read_u32(0x60004044).unwrap() & (1 << 7),
        1 << 7,
        "actual OL falling edge latches GPIO pending"
    );
    for _ in 0..200_000 {
        machine.step().unwrap();
    }
    assert_eq!(
        machine.bus.read_u32(0x60004044).unwrap() & (1 << 7),
        0,
        "ESP-IDF GPIO ISR cleared OL pending"
    );
    assert_eq!(
        machine.bus.read_u32(symbol("current_baud") as u64).unwrap(),
        38400,
        "actual OL callback caused COM2 configuration"
    );
    assert_eq!(
        machine.bus.read_u32(0x60004564).unwrap() & 0x1ff,
        9,
        "GPIO4 switched to UART1 output matrix"
    );
    assert_eq!(
        machine.bus.read_u32(0x60004178).unwrap() & 0x7f,
        0x45,
        "UART1 RX input matrix selects GPIO5"
    );
    assert_eq!(
        machine.bus.read_u32(0x60010020).unwrap() & 0x3f,
        0x1e,
        "actual ESP-IDF UART1 configured 8E1"
    );
}
