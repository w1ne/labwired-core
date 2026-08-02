// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#[test]
fn workshop_clock_uses_arduino_time_not_the_cycle_csr() {
    let source = include_str!(
        "../../../examples/esp32c3-display-workshop-arduino/esp32c3-display-workshop.ino"
    );

    for forbidden_arch_specific_timing in ["rdcycle", "asm volatile", "__asm__"] {
        assert!(
            !source.contains(forbidden_arch_specific_timing),
            "the workshop sketch must not use architecture-specific timing assembly: {forbidden_arch_specific_timing}"
        );
    }
    assert!(
        source.contains("const uint32_t now = millis();"),
        "the workshop sketch must use Arduino's portable millisecond clock"
    );
    assert!(
        source.contains("static_cast<uint32_t>(now - lastClockMillis)"),
        "the workshop sketch must compare the millisecond clock with rollover-safe subtraction"
    );
    assert!(
        source.contains("DEMO_CLOCK_INTERVAL_MS = 1000UL"),
        "the workshop clock must retain its one-second update interval"
    );
}
