/* LabWired clean-room i.MX RT500 (MIMXRT595S) boot-ROM API window.
 *
 * NOT an NXP ROM dump. The boot ROM sits at 0x0300_0000 (secure view
 * 0x1300_0000). The only part of it this provides is the three OTP entry
 * points the MCUXpresso SDK calls at fixed addresses. They are taken from the
 * SDK's devices/MIMXRT595S/drivers/fsl_clock.c (the CLOCK_EnableFroClkRange FRO
 * trim read):
 *   OTP_INIT_API      0x13007FFF  void (*)(uint32_t src_clk_freq)
 *   OTP_DEINIT_API    0x1300804D  void (*)(void)
 *   OTP_FUSE_READ_API 0x1300805D  void (*)(uint32_t addr, uint32_t *data)
 * (Thumb entry addresses: the code starts one byte lower.) The routine
 * bodies are ours.
 *
 * NOT modelled (the real ROM does these):
 * - the boot flow itself (FCB/image-header parsing, ISP, secure boot): the
 *   simulator enters the image's vector table directly;
 * - the OTP array: every fuse reads 0, i.e. a blank part. The FRO trims
 *   (FRO_SCTRIM/FRO_RDTRIM) therefore get 0, which has no simulated effect;
 * - every other ROM API (flash/IAP, runtime/power). They are not
 *   called by anything this window serves, and the bytes there read 0.
 *
 * Rebuild: crates/core/roms/rt500/build.sh (arm-none-eabi binutils). */

    .syntax unified
    .cpu cortex-m33
    .thumb

    .section .text, "ax"
    .global _rom_start
_rom_start:

    /* OTP_INIT_API: accept any source clock; nothing to set up. */
    .org 0x7FFE
    .thumb_func
otp_init:
    movs r0, #0
    bx lr

    /* OTP_DEINIT_API. */
    .org 0x804C
    .thumb_func
otp_deinit:
    movs r0, #0
    bx lr

    /* OTP_FUSE_READ_API(addr, data): *data = 0 (blank fuse). */
    .org 0x805C
    .thumb_func
otp_fuse_read:
    movs r2, #0
    str r2, [r1]
    movs r0, #0
    bx lr

    /* Pad to the end of the 64 KiB window this file maps. */
    .org 0x10000
