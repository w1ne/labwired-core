# F401 SysTick vector linker attempts

Two target-only onboarding attempts stopped during `build_firmware` because
`minimal.ld` referenced `SysTick_Handler` while the package build linked every
binary in the firmware crate, including binaries that do not define that
handler. Adding the handler reference in the smoke source did not solve the
package-wide linker-script requirement. Neither attempt booted or ran a smoke.

The final implementation keeps the shared linker script unchanged and writes a
small aligned vector table into SRAM from the Renode smoke before setting VTOR.
