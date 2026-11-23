// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F401/F411 TIM1, TIM9, TIM10 and TIM11 stay on their silicon NVIC lines.
//!
//! The four timers share vectors 24..27 (RM0368 / RM0383 vector table; Renode
//! `stm32f4.repl`): TIM1_BRK_TIM9 = 24, TIM1_UP_TIM10 = 25,
//! TIM1_TRG_COM_TIM11 = 26, TIM1_CC = 27. These descriptors once declared no
//! IRQ on any of the four, so a TIM1/TIM9..11 interrupt never pended. The
//! onboarding Black Pill smoke checks the pends at run time
//! (`tim1/9/10/11 irq ispr24=00028124 ok`); this pins every descriptor that
//! carries the same block, including the main ones no smoke exercises.

use crate::common::root;
use labwired_config::ChipDescriptor;

const DESCRIPTORS: [&str; 4] = [
    "configs/chips/stm32f401cdu6.yaml",
    "configs/chips/stm32f411ceu6.yaml",
    "configs/chips/stm32f411re.yaml",
    "configs/chips/onboarding/stm32f401cdu6-blackpill.yaml",
];

#[test]
fn f4_tim1_and_tim9_to_tim11_use_silicon_irq_lines() {
    for rel in DESCRIPTORS {
        let chip = ChipDescriptor::from_file(root(rel)).expect("chip yaml");
        let timer = |id: &str| {
            chip.peripherals
                .iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("{rel}: no {id}"))
        };
        for (id, irq) in [("tim1", 25), ("tim9", 24), ("tim10", 25), ("tim11", 26)] {
            assert_eq!(timer(id).irq, Some(irq), "{rel}: {id} irq");
        }
        let cc = timer("tim1")
            .config
            .get("capture_compare_irq")
            .and_then(|v| v.as_u64());
        assert_eq!(cc, Some(27), "{rel}: tim1 capture_compare_irq");
    }
}
