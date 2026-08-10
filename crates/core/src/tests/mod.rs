#[cfg(test)]
pub mod builtin_chip_self_contained;
#[cfg(test)]
pub mod bus_proof_matrix;
#[cfg(test)]
pub mod bus_trace_one_home;
#[cfg(test)]
pub mod cortex_m_fault_escalation;
#[cfg(test)]
pub mod cortex_m_memory_contract;
#[cfg(test)]
pub mod device_identity_one_home;
#[cfg(test)]
pub mod esp32;
#[cfg(test)]
pub mod esp32c3_i2c_waveform;
#[cfg(test)]
pub mod esp32c3_rtc_delay_loop;
#[cfg(test)]
pub mod hcsr04_event_tick_differential;
#[cfg(test)]
pub mod i2c_central_time_drive;
#[cfg(test)]
pub mod integration;
#[cfg(test)]
pub mod logic_capture;
#[cfg(test)]
pub mod logic_capture_differential;

#[cfg(test)]
pub mod bench_spi_engine;
pub mod esp32_i2c_waveform;
#[cfg(test)]
pub mod esp32s3_i2c_waveform;
pub mod esp_spi_uart_waveform;
#[cfg(test)]
pub mod machine_advance;
#[cfg(test)]
pub mod no_vacuous_test_targets;
#[cfg(test)]
pub mod nrf52;
pub mod nrf52_nvmc;
#[cfg(test)]
pub mod peripheral_reachability;
#[cfg(test)]
pub mod pre_merge_lane_covers_browser;
#[cfg(test)]
pub mod rp2040;
#[cfg(test)]
pub mod rp2040_i2c_waveform;
#[cfg(test)]
pub mod rp2040_spi_carries_a_byte;
#[cfg(test)]
pub mod rp2040_spi_waveform;
#[cfg(test)]
pub mod rp2040_uart_waveform;
#[cfg(test)]
pub mod scb_reset;
#[cfg(test)]
pub mod scheduler_lane_coverage;
#[cfg(test)]
pub mod simctl_machine;
#[cfg(test)]
pub mod spi_byte_level_golden;
#[cfg(test)]
pub mod spi_edge_sampling_lab;
#[cfg(test)]
pub mod stm32_i2c_waveform;
#[cfg(test)]
pub mod stm32_legacy_i2c_waveform;
pub mod stm32_spi_waveform;
#[cfg(test)]
pub mod stm32_uart_waveform;
#[cfg(test)]
pub mod stm32f1_bus_visibility;
#[cfg(test)]
pub mod stm32h5_spi_visibility;
#[cfg(test)]
pub mod test_cycles;
#[cfg(test)]
pub mod uart_stream_interval_differential;
#[cfg(test)]
pub mod walk_starvation_contract;
#[cfg(test)]
pub mod xtensa_memory_contract;
#[cfg(test)]
pub mod yaml_owned_base_contract;
