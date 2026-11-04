# NUCLEO-H7A3ZI-Q validation record

- ST's eStore marked the board Active and In Stock and offered Add to Cart on
  2026-11-04. The fitted STM32H7A3ZIT6Q is Active; ST listed that exact suffix
  Out of Stock separately at the same check. The board itself was purchasable.
- The profile follows ST UM2408 Rev 6 for LD1 on PB0 with SB54 ON / SB65 OFF;
  the LEDs are active high. It maps only the H7 reset-clock RCC access, GPIOB,
  2 MiB flash, and the 128 KiB DTCM used by the image. Other MCU and board
  features are not declared.
- `scripts/onboarding_smoke.sh` passed 2/2 checks at 20,000 steps: 32 recorded
  edges on GPIOB pin 0 and the expected `max_steps` stop. The committed result
  also records `unpowered_rail_assumed`; power-on and brown-out behavior are
  outside this evidence.
- No known zero-answering stub type is declared by this system. RCC and GPIO
  use the existing H7 RCC and STM32V2 GPIO models. This exact H7A3 profile has
  not been bench-checked, and the hosted catalog does not expose an H7A3 board
  or MCU target. No hosted `model_verified` verdict or physical-board capture
  is claimed.
