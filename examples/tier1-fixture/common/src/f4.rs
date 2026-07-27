//! Checks shared by the STM32F4 fixtures (F401, F407, F411).
//!
//! The F4 parts share DMA (RM0090 §10 stream controller), NVIC/EXTI, and the
//! TIM1 advanced-control timer, so these checks are parameterized by base
//! address rather than duplicated per part.
