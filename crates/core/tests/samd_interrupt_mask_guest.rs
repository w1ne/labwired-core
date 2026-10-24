// SPDX-License-Identifier: MIT
//! Owned PRIMASK control only. No NVIC/IRQ delivery, DMA or runtime qualification.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::cortex_m::configure_cortex_m, Bus, Machine};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[test]
#[ignore = "hosted source-built PRIMASK fixture; run the dedicated qualification workflow"]
fn authored_interrupt_mask_executes_and_rejects_enabled_restore() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = PathBuf::from(std::env::var("LABWIRED_MASK_FIXTURE_DIR").unwrap());
    for (case, expected) in [("positive", 0x600d), ("restore-enabled", 3)] {
        let path = directory.join(case).join("control.elf");
        let bytes = std::fs::read(&path).unwrap();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let status = labwired_loader::resolve_symbol_in_elf(&bytes, "control_status")
            .expect("owned unstripped guest status symbol") as u64;
        assert!((0x2000_0000..0x2003_0000).contains(&status));
        let chip = ChipDescriptor::from_file(root.join("configs/chips/atsamd51.yaml")).unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            "name: authored-mask-control\nchip: ../chips/atsamd51.yaml\nexternal_devices: []\n",
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        machine
            .load_firmware(&labwired_loader::load_elf(&path).unwrap())
            .unwrap();
        assert_eq!(machine.bus.read_u32(0).unwrap(), 0x2003_0000);
        assert_ne!(machine.bus.read_u32(4).unwrap() & 1, 0);
        machine.bus.write_u32(status, 0xa5a5_a5a5).unwrap();
        let mut saw_zero = false;
        let mut saw_entry = false;
        let mut reached = false;
        let mut steps = 0;
        for step in 1..=100_000 {
            machine.step().expect("guest instruction must not fault");
            let observed = machine.bus.read_u32(status).unwrap();
            saw_zero |= observed == 0;
            saw_entry |= observed == 0x100;
            if observed != 0 && observed != 0x100 && observed != 0xa5a5_a5a5 {
                assert_eq!(observed, expected, "first guest result for {case}");
                reached = true;
                steps = step;
                break;
            }
        }
        assert!(
            saw_zero && saw_entry && reached,
            "bounded startup/result for {case}"
        );
        println!("SAMD_INTERRUPT_MASK case={case} elf_sha256={hash} status={expected:#x} steps={steps} bss_clear=true entry=true");
    }
}
