#[cfg(test)]
pub mod builtin_chip_self_contained;
#[cfg(test)]
pub mod bus_trace_one_home;
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
pub mod machine_advance;
#[cfg(test)]
pub mod nrf52;
#[cfg(test)]
pub mod peripheral_reachability;
#[cfg(test)]
pub mod rp2040;
#[cfg(test)]
pub mod scb_reset;
#[cfg(test)]
pub mod stm32_spi_waveform;
#[cfg(test)]
pub mod test_cycles;
#[cfg(test)]
pub mod walk_starvation_contract;
