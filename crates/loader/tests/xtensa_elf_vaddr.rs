use labwired_core::Arch;

fn put16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn xtensa_elf_with_distinct_lma_vma() -> Vec<u8> {
    const EHDR: usize = 52;
    const PHDR: usize = 32;
    const DATA: usize = 0x100;
    let mut elf = vec![0_u8; DATA + 8];
    elf[0..4].copy_from_slice(b"\x7fELF");
    elf[4] = 1;
    elf[5] = 1;
    elf[6] = 1;
    put16(&mut elf, 16, 2);
    put16(&mut elf, 18, 94);
    put32(&mut elf, 20, 1);
    put32(&mut elf, 24, 0x400d_0000);
    put32(&mut elf, 28, EHDR as u32);
    put16(&mut elf, 40, EHDR as u16);
    put16(&mut elf, 42, PHDR as u16);
    put16(&mut elf, 44, 2);

    for (index, (offset, vaddr, paddr, flags)) in [
        (DATA as u32, 0x3f40_0020, 0x3c80_0020, 4_u32),
        ((DATA + 4) as u32, 0x3ffb_0000, 0x3fce_0000, 6_u32),
    ]
    .into_iter()
    .enumerate()
    {
        let at = EHDR + index * PHDR;
        put32(&mut elf, at, 1);
        put32(&mut elf, at + 4, offset);
        put32(&mut elf, at + 8, vaddr);
        put32(&mut elf, at + 12, paddr);
        put32(&mut elf, at + 16, 4);
        put32(&mut elf, at + 20, 4);
        put32(&mut elf, at + 24, flags);
        put32(&mut elf, at + 28, 4);
    }
    elf[DATA..DATA + 8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    elf
}

#[test]
fn loader_uses_xtensa_runtime_vma() {
    let image = labwired_loader::load_elf_bytes(&xtensa_elf_with_distinct_lma_vma()).unwrap();
    assert_eq!(image.arch, Arch::XtensaLx7);
    assert_eq!(image.segments.len(), 2);
    assert_eq!(image.segments[0].start_addr, 0x3f40_0020);
    assert_eq!(image.segments[1].start_addr, 0x3ffb_0000);
}
