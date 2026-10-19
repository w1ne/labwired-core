//! Host/loader writes are not CPU stores: `Machine::load_firmware` must not
//! be rejected or recorded by the CPU-write policy gate or an armed debug
//! watch.
//!
//! The regression: a boot-alias image (ELF linked at `0x0`) on a chip whose
//! flash lives at `0x0800_0000` cannot use the direct `flash`/`ram`/`extra_mem`
//! placement (those key on the physical base), so it falls through to the
//! byte-by-byte bus walk in `Machine::load_firmware`. That walk went through
//! the ordinary store path, so `flash.cpu_writes: fault` rejected the load and
//! a debug watch latched — both of which are about CPU stores.

use crate::bus::SystemBus;
use crate::memory::LinearMemory;
use crate::system::cortex_m::configure_cortex_m;
use crate::{Bus, Machine};

fn fixture(name: &str) -> std::path::PathBuf {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates/
    path.pop(); // workspace root
    path.push("tests/fixtures");
    path.push(name);
    path
}

/// The same PT_LOAD → `ProgramImage` mapping `labwired_loader::load_elf` does.
/// Parsed here rather than through the loader crate: `core` dev-depends on
/// `labwired-loader`, which depends on `core`, so the loader's `ProgramImage`
/// is a second crate instance with an incompatible type.
fn load_elf_image(path: &std::path::Path) -> crate::memory::ProgramImage {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let elf = goblin::elf::Elf::parse(&bytes).expect("parse ELF");
    let mut image = crate::memory::ProgramImage::new(elf.entry, crate::Arch::Arm);
    for ph in &elf.program_headers {
        if ph.p_type != goblin::elf::program_header::PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let off = ph.p_offset as usize;
        let size = ph.p_filesz as usize;
        image.add_segment(ph.p_paddr, bytes[off..off + size].to_vec());
    }
    image
}

#[test]
fn boot_alias_loader_write_bypasses_policy_and_watch() {
    // Linked at 0x0/0x400 with flash at 0x0800_0000 and `flash_boot_alias` on,
    // this image takes the peripheral/alias byte-walk fallback.
    let image = load_elf_image(&fixture("uart-ok-thumbv7m.elf"));

    let mut bus = SystemBus::new();
    bus.flash = LinearMemory::new_erased(64 * 1024, 0x0800_0000);
    bus.ram = LinearMemory::new_erased(16 * 1024, 0x2000_0000);
    bus.flash_boot_alias = true;
    bus.memory_policy.flash = crate::bus::CpuWritePolicy::Fault;
    // Watch the boot-alias window the loader writes through, and the physical
    // flash address the CPU store below targets. A store must latch the latter
    // but neither may latch during the load.
    assert!(bus.add_debug_watch(0, 0x1000, false, true));
    assert!(bus.add_debug_watch(0x0800_1000, 4, false, true));

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    machine
        .load_firmware(&image)
        .expect("a host loader write must not be faulted by the CPU-write policy");

    assert_eq!(
        machine.bus.memory_policy.violations_flash, 0,
        "the loader must not record a policy violation"
    );
    assert_eq!(
        machine.bus.take_debug_watch_hit(),
        None,
        "the loader must not latch an armed store watch"
    );
    // The .text bytes landed at the alias (first word of the fixture's .text).
    assert_eq!(machine.bus.read_u32(0x400).unwrap(), 0xf82c_f000);

    // Control: a CPU store into flash is still gated and still latches.
    let err = machine.bus.write_u8(0x0800_1000, 0xAB).unwrap_err();
    assert!(matches!(
        err,
        crate::SimulationError::MemoryViolation(0x0800_1000)
    ));
    assert_eq!(machine.bus.memory_policy.violations_flash, 1);
    assert_eq!(
        machine.bus.take_debug_watch_hit(),
        Some(0x0800_1000),
        "the CPU store must latch the flash watch"
    );
}
