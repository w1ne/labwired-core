/* RP2350 DCP canned-sequence harness (fixture for
 * crates/core/tests/rp2350_dcp_canned.rs).
 *
 * Every t_* body below is the pico-sdk double/float routine body verbatim:
 * the dcp_*_m macro invocation from pico_double/double_aeabi_dcp.S (and the
 * float ones from pico_float/float_aeabi_dcp.S), expanded through the SDK's
 * own hardware_dcp/dcp_instr.inc.S + dcp_canned.inc.S (copied unmodified
 * from pico-sdk at the commit in PICO_SDK_COMMIT, BSD-3-Clause). The test
 * sets r0..r3, points lr at a `b .` sentinel and runs the function.
 *
 * t_dadd_saving is __aeabi_dadd with the SDK's saving_func wrapper and
 * generic_save_state/generic_restore_state copied verbatim: PCMP tests the
 * engaged flag, saves the six state words, runs the add, restores them.
 *
 * Rebuild: ./build.sh (arm-none-eabi binutils).
 */
    .syntax unified
    .cpu cortex-m33
    .thumb

#include "dcp_instr.inc.S"
#include "dcp_canned.inc.S"

    .section .vectors, "a"
    .word 0x20080000
    .word t_park + 1

    .text
    .macro fn name
    .global \name
    .type \name, %function
    .thumb_func
\name:
    .endm

fn t_park
    b t_park

fn t_dadd
    dcp_dadd_m r0,r1,r0,r1,r2,r3
    bx lr
fn t_dsub
    dcp_dsub_m r0,r1,r0,r1,r2,r3
    bx lr
fn t_dmul
    push {r4,r14}
    dcp_dmul_m r0,r1,r0,r1,r2,r3,r0,r1,r2,r3,r4,r12,r14
    pop {r4,pc}
fn t_ddiv
    dcp_ddiv_m r0,r1,r0,r1,r2,r3,r0,r1,r2,r3,r12
    bx lr
fn t_ddiv_fast
    dcp_ddiv_fast_m r0,r1,r0,r1,r2,r3,r0,r1,r2,r3,r12
    bx lr
fn t_dsqrt
    dcp_dsqrt_m r0,r1,r0,r1,r0,r1,r2,r3,r12
    bx lr
fn t_dsqrt_fast
    dcp_dsqrt_fast_m r0,r1,r0,r1,r0,r1,r2,r3,r12
    bx lr
/* __aeabi_cdcmple: flags in APSR; returned as r0 = APSR for the test. */
fn t_dcmp_flags
    dcp_dcmp_m apsr_nzcv,r0,r1,r2,r3
    mrs r0, apsr
    bx lr
/* __aeabi_dcmpeq / __aeabi_dcmpun bodies. */
fn t_dcmpeq
    dcp_dcmp_m r0,r0,r1,r2,r3
    ubfx r0, r0, #30, #1
    bx lr
fn t_dcmpun
    dcp_dcmp_m r0,r0,r1,r2,r3
    ubfx r0, r0, #28, #1
    bx lr
fn t_i2d
    dcp_int2double_m r0,r1,r0
    bx lr
fn t_ui2d
    dcp_uint2double_m r0,r1,r0
    bx lr
fn t_d2iz
    dcp_double2int_m r0,r0,r1
    bx lr
fn t_d2uiz
    dcp_double2uint_m r0,r0,r1
    bx lr
fn t_d2i_r
    dcp_double2int_r_m r0,r0,r1
    bx lr
fn t_d2f
    dcp_double2float_m r0,r0,r1
    bx lr
fn t_f2d
    dcp_float2double_m r0,r1,r0
    bx lr
fn t_fadd
    dcp_fadd_m r0,r0,r1
    bx lr
fn t_fmul
    dcp_fmul_m r0,r0,r1,r0,r1
    bx lr
fn t_fdiv
    dcp_fdiv_m r0,r0,r1,r0,r1,r2,r3
    bx lr
fn t_fsqrt
    dcp_fsqrt_m r0,r0,r0,r1,r2,r3
    bx lr
fn t_i2f
    dcp_int2float_m r0,r0
    bx lr
fn t_f2iz
    dcp_float2int_m r0,r0
    bx lr

/* Leave a dsub half-done (operands loaded, ADD0 issued: engaged). */
fn t_engage_dsub
    WXUP r0,r1
    WYUP r2,r3
    ADD0
    bx lr
/* Finish the dsub left by t_engage_dsub. */
fn t_finish_dsub
    SUB1
    NRDD
    RDDS r0,r1
    bx lr

/* ---- pico-sdk saving_func wrapper __aeabi_dadd, verbatim ---- */
.p2align 2
1:
  push {lr}
  bl generic_save_state
  b 1f
fn t_dadd_saving
  /* SDK: `PCMP apsr_nzcv`. GNU as 2.46 rejects apsr_nzcv for mrc2; Rt=15
   * is the same encoding (MRC Rt=15 writes APSR.NZCV). */
  PCMP r15
  bmi 1b
1:
  dcp_dadd_m r0,r1,r0,r1,r2,r3
  bx lr

.thumb_func
generic_save_state:
  sub sp, #24
  push {r0, r1}
  PXMD r0, r1
  strd r0, r1, [sp, #8 + 0]
  PYMD r0, r1
  strd r0, r1, [sp, #8 + 8]
  REFD r0, r1
  strd r0, r1, [sp, #8 + 16]
  pop {r0, r1}
  blx lr
.thumb_func
generic_restore_state:
  pop {r12, r14}
  WXMD r12, r14
  pop {r12, r14}
  WYMD r12, r14
  pop {r12, r14}
  WEFD r12, r14
  pop {pc}
