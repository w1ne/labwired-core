#![cfg_attr(not(test), no_std)]

pub mod ble;
pub mod isotp;
pub mod mcp2515;
pub mod obd2;
pub mod ssd1306;
pub mod state;

pub use isotp::{IsoTpEvent, VinReassembler};
pub use obd2::{
    clear_dtcs_request, decode_clear_dtcs, decode_coolant, decode_dtcs, decode_rpm, decode_speed,
    decode_supported_pids, mode01_request, read_dtcs_request, vin_request, CanFrame, Dtc, DtcList,
    DtcSystem, Error, FLOW_CONTROL_ID, REQUEST_ID, RESPONSE_ID,
};
pub use state::{flags, live, AcquisitionFailure, PollSchedule, ScannerState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CycleOutputs {
    pub ble_payload: [u8; ble::PAYLOAD_LEN],
    pub display: ssd1306::DisplayView,
}

/// Applies the device result first, then derives both externally visible views.
pub fn finalize_cycle_outputs(state: &mut ScannerState, device_failed: bool) -> CycleOutputs {
    if device_failed {
        state.apply_failure(AcquisitionFailure::Device);
    }
    CycleOutputs {
        ble_payload: ble::encode_manufacturer_payload(state),
        display: ssd1306::DisplayView::from_state(state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ble::{encode_manufacturer_payload, Radio, RADIO_DMA_STATIC},
        mcp2515::{
            decode_rx_registers, tx_decision, validate_frame, TxDecision, LOAD_TXB0,
            MCP_500K_16MHZ_CNF, RTS_TXB0, SPIM_DMA_STATIC, SPIM_EVENTS_END, SPIM_EVENTS_STOPPED,
        },
        ssd1306::{DisplayView, TWIM_DMA_STATIC, TWIM_EVENTS_ERROR, TWIM_EVENTS_STOPPED},
        state::{live, AcquisitionFailure, PollSchedule},
    };

    #[test]
    fn cycle_outputs_are_derived_from_final_device_result() {
        let mut success = ScannerState::new();
        let success_outputs = finalize_cycle_outputs(&mut success, false);
        assert_eq!(
            success_outputs.ble_payload[1] & flags::DEVICE_ERROR as u8,
            0
        );
        assert_eq!(success_outputs.display.status.as_bytes(), b"STALE");

        let mut failed = ScannerState::new();
        let failed_outputs = finalize_cycle_outputs(&mut failed, true);
        assert_eq!(
            failed_outputs.ble_payload[1] & flags::DEVICE_ERROR as u8,
            0x80
        );
        assert_eq!(failed_outputs.display.status.as_bytes(), b"DEV ERR");
        assert_eq!(failed_outputs.ble_payload[1], failed.status_flags as u8);
    }

    #[test]
    fn txb0_status_requires_real_completion_and_detects_errors() {
        assert_eq!(LOAD_TXB0, 0x40);
        assert_eq!(RTS_TXB0, 0x81);
        assert_eq!(tx_decision(0x08, 0), TxDecision::Pending);
        assert_eq!(tx_decision(0, 0x04), TxDecision::Complete);
        for error in [0x10, 0x20, 0x40] {
            assert_eq!(tx_decision(error, 0), TxDecision::Failed);
        }
        assert_eq!(tx_decision(0, 0), TxDecision::Pending);
    }

    #[test]
    fn malformed_rx_dlc_is_typed_and_next_valid_decode_progresses() {
        let mut registers = [0u8; 13];
        registers[4] = 9;
        assert_eq!(
            decode_rx_registers(&registers),
            Err(mcp2515::Error::InvalidFrame)
        );
        registers[4] = 8;
        registers[5] = 0xaa;
        assert_eq!(decode_rx_registers(&registers).unwrap().data[0], 0xaa);
    }

    #[test]
    fn device_error_is_stable_eighth_wire_bit_and_displayed() {
        let mut state = ScannerState::new();
        state.apply_failure(AcquisitionFailure::Device);
        assert_eq!(
            encode_manufacturer_payload(&state)[1],
            (flags::STALE | flags::DEVICE_ERROR) as u8
        );
        assert_eq!(
            DisplayView::from_state(&state).status.as_bytes(),
            b"DEV ERR"
        );
        state.status_flags |= flags::TIMEOUT | flags::CAN_CONFIG_ERROR;
        assert_eq!(
            DisplayView::from_state(&state).status.as_bytes(),
            b"STALE TO CAN DEV ERR"
        );
    }

    #[test]
    fn snapshot_sequence_protocol_uses_odd_then_even() {
        assert_eq!(state::snapshot_sequence_pair(0), (1, 2));
        assert_eq!(state::snapshot_sequence_pair(2), (3, 4));
        assert_eq!(state::snapshot_sequence_pair(u32::MAX - 1), (u32::MAX, 0));
    }

    #[test]
    fn radio_is_singleton_with_static_packet_storage() {
        const {
            assert!(RADIO_DMA_STATIC);
        }
        let first = Radio::take();
        assert!(first.is_some());
        assert!(Radio::take().is_none());
    }

    #[test]
    fn polling_stays_on_discovery_then_starts_at_rpm() {
        let mut schedule = PollSchedule::new();
        assert_eq!(schedule.request_pid(), 0x00);
        schedule.discovery_failed();
        assert_eq!(schedule.request_pid(), 0x00);
        schedule.discovery_failed();
        assert_eq!(schedule.request_pid(), 0x00);
        schedule.discovery_succeeded();
        assert_eq!(schedule.request_pid(), 0x0c);
        schedule.live_attempted();
        assert_eq!(schedule.request_pid(), 0x0d);
        schedule.live_attempted();
        assert_eq!(schedule.request_pid(), 0x05);
    }

    #[test]
    fn reviewed_mmio_offsets_and_can_timing_are_exact() {
        assert_eq!(SPIM_EVENTS_STOPPED, 0x104);
        assert_eq!(SPIM_EVENTS_END, 0x118);
        assert_eq!(TWIM_EVENTS_STOPPED, 0x104);
        assert_eq!(TWIM_EVENTS_ERROR, 0x124);
        assert_eq!(MCP_500K_16MHZ_CNF, [0x00, 0xbc, 0x01]);
        const {
            assert!(SPIM_DMA_STATIC);
            assert!(TWIM_DMA_STATIC);
        }
    }

    #[test]
    fn mcp_rejects_invalid_standard_frames_before_transfer() {
        let mut frame = CanFrame {
            id: 0x800,
            len: 8,
            data: [0; 8],
        };
        assert_eq!(validate_frame(&frame), Err(mcp2515::Error::InvalidFrame));
        frame.id = 0x7ff;
        frame.len = 9;
        assert_eq!(validate_frame(&frame), Err(mcp2515::Error::InvalidFrame));
    }

    #[test]
    fn failed_live_pid_stays_invalid_until_that_pid_recovers() {
        let mut state = ScannerState::new();
        state.record_rpm(3000);
        state.record_speed(88);
        state.record_coolant(90);
        assert!(state.has_all(flags::CONNECTED));
        state.invalidate_live(live::RPM, AcquisitionFailure::Timeout);
        state.record_speed(89);
        assert_eq!(state.live_valid, live::SPEED | live::COOLANT);
        assert!(!state.has_any(flags::CONNECTED));
        assert!(state.has_all(flags::STALE | flags::TIMEOUT));
        state.record_rpm(3100);
        assert!(state.has_all(flags::CONNECTED));
        assert!(!state.has_any(flags::STALE | flags::TIMEOUT));
    }

    #[test]
    fn missing_required_pid_never_connects_and_sets_error() {
        let mut state = ScannerState::new();
        // PID 0C only; speed and coolant are absent.
        assert!(!state.accept_supported_pids(1 << (32 - 0x0c)));
        state.record_rpm(3000);
        assert_eq!(state.required_live, live::ALL);
        assert!(!state.has_any(flags::CONNECTED));
        assert!(state.has_all(flags::MALFORMED | flags::STALE));
        assert_eq!(
            encode_manufacturer_payload(&state)[1] & flags::CONNECTED as u8,
            0
        );
    }

    #[test]
    fn display_combines_status_exactly_without_truncation() {
        let mut state = ScannerState::new();
        state.status_flags = flags::STALE
            | flags::TIMEOUT
            | flags::MALFORMED
            | flags::RX_OVERFLOW
            | flags::CAN_CONFIG_ERROR;
        assert_eq!(
            DisplayView::from_state(&state).status.as_bytes(),
            b"STALE TIMEOUT CAN ERR"
        );
        state.status_flags &= !flags::CAN_CONFIG_ERROR;
        assert_eq!(
            DisplayView::from_state(&state).status.as_bytes(),
            b"STALE TIMEOUT"
        );
    }

    #[test]
    fn successful_clear_dtcs_updates_next_snapshot() {
        let mut state = ScannerState::new();
        state.update_dtc_count(2);
        state.clear_dtcs();
        assert_eq!(state.dtc_count, 0);
        assert!(!state.has_any(flags::DTC_PRESENT));
        assert_eq!(encode_manufacturer_payload(&state)[6], 0);
    }

    #[test]
    fn partial_live_samples_remain_initializing_and_invalid_fields_are_hidden() {
        let mut state = ScannerState::new();
        state.mark_fresh();
        assert!(!state.has_any(flags::CONNECTED));
        state.record_rpm(3000);
        assert_eq!(state.live_valid, live::RPM);
        assert!(!state.has_any(flags::CONNECTED));
        assert!(state.has_any(flags::STALE));
        let view = DisplayView::from_state(&state);
        assert_eq!(view.lines[0].as_bytes(), b"RPM 3000");
        assert_eq!(view.lines[1].as_bytes(), b"SPD -- km/h");
        assert_eq!(view.lines[2].as_bytes(), b"TEMP -- C");
        assert_eq!(view.status.as_bytes(), b"STALE");
        assert_eq!(
            encode_manufacturer_payload(&state)[1] & flags::CONNECTED as u8,
            0
        );

        state.record_speed(88);
        assert!(!state.has_any(flags::CONNECTED));
        state.record_coolant(90);
        assert!(state.has_all(flags::CONNECTED));
        assert!(!state.has_any(flags::STALE));
        assert_eq!(state.live_valid, live::ALL);
    }

    #[test]
    fn acquisition_failures_map_to_persistent_state_without_erasing_readings() {
        let mut state = ScannerState::new();
        state.record_rpm(3000);
        state.apply_failure(AcquisitionFailure::Timeout);
        assert_eq!(state.rpm, 3000);
        assert!(state.has_all(flags::TIMEOUT | flags::STALE));
        state.apply_failure(AcquisitionFailure::Malformed);
        assert!(state.has_all(flags::MALFORMED | flags::STALE));
        state.apply_failure(AcquisitionFailure::Overflow);
        assert!(state.has_all(flags::RX_OVERFLOW | flags::STALE));
        state.apply_failure(AcquisitionFailure::Configuration);
        assert!(state.has_all(flags::CAN_CONFIG_ERROR | flags::STALE));
    }

    #[test]
    fn ble_payload_has_versioned_exact_layout() {
        let mut state = ScannerState::new();
        state.record_rpm(3000);
        state.record_speed(88);
        state.record_coolant(90);
        state.update_dtc_count(2);
        state.generation = 0x1234;
        assert_eq!(
            encode_manufacturer_payload(&state),
            [1, 0b0000_0101, 0xb8, 0x0b, 88, 90, 2, 0x34, 0x12]
        );
    }

    #[test]
    fn ble_coolant_is_celsius_and_clamped() {
        let mut state = ScannerState::new();
        state.coolant_c = 90;
        assert_eq!(encode_manufacturer_payload(&state)[5], 90);
        state.coolant_c = 0;
        assert_eq!(encode_manufacturer_payload(&state)[5], 0);
        state.coolant_c = -100;
        assert_eq!(encode_manufacturer_payload(&state)[5], 0);
        state.coolant_c = 300;
        assert_eq!(encode_manufacturer_payload(&state)[5], 255);
    }

    #[test]
    fn ble_maps_all_status_bits_without_silent_truncation() {
        let mut state = ScannerState::new();
        state.status_flags = flags::CONNECTED
            | flags::STALE
            | flags::DTC_PRESENT
            | flags::TIMEOUT
            | flags::MALFORMED
            | flags::RX_OVERFLOW
            | flags::CAN_CONFIG_ERROR;
        assert_eq!(encode_manufacturer_payload(&state)[1], 0x7f);
    }

    #[test]
    fn display_view_formats_sample_without_allocation() {
        let mut state = ScannerState::new();
        state.record_rpm(3000);
        state.record_speed(88);
        state.record_coolant(90);
        state.update_dtc_count(2);
        let view = DisplayView::from_state(&state);
        assert_eq!(view.lines[0].as_bytes(), b"RPM 3000");
        assert_eq!(view.lines[1].as_bytes(), b"SPD 88 km/h");
        assert_eq!(view.lines[2].as_bytes(), b"TEMP 90 C");
        assert_eq!(view.lines[3].as_bytes(), b"DTC 2");
    }

    #[test]
    fn display_view_reports_stale_timeout_and_can_error() {
        let mut state = ScannerState::new();
        state.record_rpm(0);
        state.record_speed(0);
        state.record_coolant(0);
        state.status_flags |= flags::STALE | flags::TIMEOUT | flags::CAN_CONFIG_ERROR;
        state.status_flags &= !flags::CONNECTED;
        let view = DisplayView::from_state(&state);
        assert_eq!(view.status.as_bytes(), b"STALE TIMEOUT CAN ERR");
    }

    #[test]
    fn display_view_handles_numeric_extremes_without_overflow() {
        let mut state = ScannerState::new();
        state.record_rpm(u16::MAX);
        state.record_speed(u8::MAX);
        state.record_coolant(i16::MIN);
        state.dtc_count = u8::MAX;
        let view = DisplayView::from_state(&state);
        assert_eq!(view.lines[0].as_bytes(), b"RPM 65535");
        assert_eq!(view.lines[1].as_bytes(), b"SPD 255 km/h");
        assert_eq!(view.lines[2].as_bytes(), b"TEMP -32768 C");
        assert_eq!(view.lines[3].as_bytes(), b"DTC 255");
    }

    fn response(data: [u8; 8]) -> CanFrame {
        CanFrame {
            id: RESPONSE_ID,
            len: 8,
            data,
        }
    }

    #[test]
    fn exact_request_frames() {
        assert_eq!(mode01_request(0x0c).data, [2, 1, 0x0c, 0, 0, 0, 0, 0]);
        assert_eq!(read_dtcs_request().data, [1, 3, 0, 0, 0, 0, 0, 0]);
        assert_eq!(clear_dtcs_request().data, [1, 4, 0, 0, 0, 0, 0, 0]);
        assert_eq!(vin_request().data, [2, 9, 2, 0, 0, 0, 0, 0]);
        assert_eq!(mode01_request(0).id, REQUEST_ID);
    }

    #[test]
    fn mode01_decoders_and_supported_bitmap() {
        assert_eq!(
            decode_rpm(&response([4, 0x41, 0x0c, 0x2e, 0xe0, 0, 0, 0])),
            Ok(3000)
        );
        assert_eq!(
            decode_speed(&response([3, 0x41, 0x0d, 88, 0, 0, 0, 0])),
            Ok(88)
        );
        assert_eq!(
            decode_coolant(&response([3, 0x41, 5, 130, 0, 0, 0, 0])),
            Ok(90)
        );
        assert_eq!(
            decode_supported_pids(&response([6, 0x41, 0, 0x80, 0, 0, 1, 0])),
            Ok(0x8000_0001)
        );
    }

    #[test]
    fn mode01_rejects_wrong_metadata_and_negative_responses() {
        let mut frame = response([4, 0x41, 0x0c, 0x2e, 0xe0, 0, 0, 0]);
        frame.id = 0x7e9;
        assert_eq!(decode_rpm(&frame), Err(Error::WrongId));
        assert_eq!(
            decode_rpm(&response([4, 0x42, 0x0c, 0, 0, 0, 0, 0])),
            Err(Error::UnsupportedService)
        );
        assert_eq!(
            decode_rpm(&response([4, 0x41, 0x0d, 0, 0, 0, 0, 0])),
            Err(Error::UnsupportedPid)
        );
        assert_eq!(
            decode_rpm(&response([3, 0x41, 0x0c, 0, 0, 0, 0, 0])),
            Err(Error::ShortPayload)
        );
        assert_eq!(
            decode_rpm(&response([3, 0x7f, 1, 0x12, 0, 0, 0, 0])),
            Err(Error::NegativeResponse {
                service: 1,
                nrc: 0x12
            })
        );
        assert_eq!(
            decode_rpm(&CanFrame {
                id: RESPONSE_ID,
                len: 9,
                data: [0; 8]
            }),
            Err(Error::InvalidLength)
        );
        assert_eq!(
            decode_rpm(&response([0x14, 0x41, 0x0c, 0, 0, 0, 0, 0])),
            Err(Error::Malformed)
        );
        assert_eq!(
            decode_supported_pids(&response([7, 0x41, 0, 0x80, 0, 0, 1, 0])),
            Err(Error::InvalidLength)
        );
        assert_eq!(
            decode_rpm(&response([5, 0x41, 0x0c, 0x2e, 0xe0, 0, 0, 0])),
            Err(Error::InvalidLength)
        );
        assert_eq!(
            decode_speed(&response([4, 0x41, 0x0d, 88, 0, 0, 0, 0])),
            Err(Error::InvalidLength)
        );
        assert_eq!(
            decode_coolant(&response([4, 0x41, 5, 130, 0, 0, 0, 0])),
            Err(Error::InvalidLength)
        );
    }

    #[test]
    fn mode03_dtcs_decode_sae_mapping_and_padding() {
        let result = decode_dtcs(&response([7, 0x43, 0x01, 0x33, 0xc1, 0x23, 0, 0])).unwrap();
        assert_eq!(result.count, 2);
        assert_eq!(result.dtcs[0].ascii(), *b"P0133");
        assert_eq!(result.dtcs[1].ascii(), *b"U0123");
        assert_eq!(
            decode_clear_dtcs(&response([1, 0x44, 0, 0, 0, 0, 0, 0])),
            Ok(())
        );
        assert_eq!(
            decode_dtcs(&response([3, 0x7f, 3, 0x11, 0, 0, 0, 0])),
            Err(Error::NegativeResponse {
                service: 3,
                nrc: 0x11
            })
        );
        assert_eq!(
            decode_clear_dtcs(&response([1, 0x43, 0, 0, 0, 0, 0, 0])),
            Err(Error::UnsupportedService)
        );

        let hex_dtcs = decode_dtcs(&response([5, 0x43, 0x01, 0xaf, 0xca, 0xbc, 0, 0])).unwrap();
        assert_eq!(hex_dtcs.dtcs[0].ascii(), *b"P01AF");
        assert_eq!(hex_dtcs.dtcs[1].ascii(), *b"U0ABC");
    }

    #[test]
    fn vin_reassembly_returns_flow_control_and_exact_vin() {
        let mut rx = VinReassembler::new();
        let ff = response([0x10, 20, 0x49, 2, 1, b'1', b'H', b'G']);
        let flow = rx.push(&ff).unwrap();
        assert_eq!(
            flow,
            IsoTpEvent::FlowControl(CanFrame {
                id: FLOW_CONTROL_ID,
                len: 8,
                data: [0x30, 0, 0, 0, 0, 0, 0, 0]
            })
        );
        assert_eq!(
            rx.push(&response([0x21, b'B', b'H', b'4', b'1', b'J', b'X', b'M'])),
            Ok(IsoTpEvent::Pending)
        );
        assert_eq!(
            rx.push(&response([0x22, b'N', b'1', b'0', b'9', b'1', b'8', b'6'])),
            Ok(IsoTpEvent::Complete(*b"1HGBH41JXMN109186"))
        );
    }

    #[test]
    fn vin_reassembly_rejects_sequence_oversize_and_timeout_without_stale_data() {
        let mut rx = VinReassembler::new();
        assert_eq!(
            rx.push(&response([0x10, 21, 0x49, 2, 1, b'X', b'X', b'X'])),
            Err(Error::Oversize)
        );
        assert_eq!(
            rx.push(&response([0x10, 19, 0x49, 2, 1, b'X', b'X', b'X'])),
            Err(Error::Malformed)
        );
        assert_eq!(
            rx.push(&response([0x10, 20, 0x49, 2, 1, b'1', b'H', b'G'])),
            Ok(IsoTpEvent::FlowControl(CanFrame {
                id: FLOW_CONTROL_ID,
                len: 8,
                data: [0x30, 0, 0, 0, 0, 0, 0, 0]
            }))
        );
        assert_eq!(
            rx.push(&response([0x22, 0, 0, 0, 0, 0, 0, 0])),
            Err(Error::Sequence)
        );
        assert_eq!(
            rx.push(&response([0x21, 0, 0, 0, 0, 0, 0, 0])),
            Err(Error::UnexpectedFrame)
        );
        assert_eq!(
            rx.push(&response([0x10, 20, 0x49, 2, 1, b'1', b'H', b'G'])),
            Ok(IsoTpEvent::FlowControl(CanFrame {
                id: FLOW_CONTROL_ID,
                len: 8,
                data: [0x30, 0, 0, 0, 0, 0, 0, 0]
            }))
        );
        assert_eq!(rx.timeout(), Err(Error::Incomplete));
        rx.reset();
        assert_eq!(rx.timeout(), Ok(()));
    }

    #[test]
    fn vin_reassembly_validates_first_frame_header_before_flow_control() {
        let mut rx = VinReassembler::new();
        assert_eq!(
            rx.push(&response([0x10, 20, 0x48, 2, 1, b'1', b'H', b'G'])),
            Err(Error::UnsupportedService)
        );
        assert_eq!(
            rx.push(&response([0x10, 20, 0x49, 3, 1, b'1', b'H', b'G'])),
            Err(Error::UnsupportedPid)
        );
        assert_eq!(
            rx.push(&response([0x10, 20, 0x49, 2, 2, b'1', b'H', b'G'])),
            Err(Error::Malformed)
        );

        rx.push(&response([0x10, 20, 0x49, 2, 1, b'1', b'H', b'G']))
            .unwrap();
        rx.push(&response([0x21, b'B', b'H', b'4', b'1', b'J', b'X', b'M']))
            .unwrap();
        assert_eq!(
            rx.push(&response([0x22, b'N', b'1', b'0', b'9', b'1', b'8', b'6',])),
            Ok(IsoTpEvent::Complete(*b"1HGBH41JXMN109186"))
        );
    }

    #[test]
    fn vin_reassembly_handles_mode09_negative_single_frames() {
        let mut rx = VinReassembler::new();
        let negative = CanFrame {
            id: RESPONSE_ID,
            len: 4,
            data: [3, 0x7f, 9, 0x11, 0, 0, 0, 0],
        };
        assert_eq!(
            rx.push(&negative),
            Err(Error::NegativeResponse {
                service: 9,
                nrc: 0x11
            })
        );
        let wrong_service = CanFrame {
            id: RESPONSE_ID,
            len: 4,
            data: [3, 0x7f, 1, 0x11, 0, 0, 0, 0],
        };
        assert_eq!(rx.push(&wrong_service), Err(Error::UnsupportedService));
        assert_eq!(
            rx.push(&response([2, 0x7f, 9, 0x11, 0, 0, 0, 0])),
            Err(Error::InvalidLength)
        );
        assert_eq!(
            rx.push(&response([4, 0x7f, 9, 0x11, 0, 0, 0, 0])),
            Err(Error::InvalidLength)
        );
        for dlc in [1, 2, 3, 5] {
            let truncated_or_padded = CanFrame {
                id: RESPONSE_ID,
                len: dlc,
                data: [3, 0x7f, 9, 0x11, 0, 0, 0, 0],
            };
            assert_eq!(rx.push(&truncated_or_padded), Err(Error::InvalidLength));
        }
    }

    #[test]
    fn vin_reassembly_rejects_duplicate_unexpected_and_bad_length_frames() {
        let mut rx = VinReassembler::new();
        let positive_single_frame = CanFrame {
            id: RESPONSE_ID,
            len: 4,
            data: [3, 0x49, 2, 1, 0, 0, 0, 0],
        };
        assert_eq!(rx.push(&positive_single_frame), Err(Error::UnexpectedFrame));
        assert_eq!(
            rx.push(&response([0x30, 0, 0, 0, 0, 0, 0, 0])),
            Err(Error::UnexpectedFrame)
        );

        rx.push(&response([0x10, 20, 0x49, 2, 1, b'1', b'H', b'G']))
            .unwrap();
        rx.push(&response([0x21, b'B', b'H', b'4', b'1', b'J', b'X', b'M']))
            .unwrap();
        assert_eq!(
            rx.push(&response([0x21, 0, 0, 0, 0, 0, 0, 0])),
            Err(Error::Sequence)
        );

        rx.push(&response([0x10, 20, 0x49, 2, 1, b'1', b'H', b'G']))
            .unwrap();
        let mut short_cf = response([0x21, b'B', b'H', b'4', b'1', b'J', b'X', b'M']);
        short_cf.len = 7;
        assert_eq!(rx.push(&short_cf), Err(Error::InvalidLength));

        rx.push(&response([0x10, 20, 0x49, 2, 1, b'1', b'H', b'G']))
            .unwrap();
        let mut overlong_cf = response([0x21, b'B', b'H', b'4', b'1', b'J', b'X', b'M']);
        overlong_cf.len = 9;
        assert_eq!(rx.push(&overlong_cf), Err(Error::InvalidLength));

        rx.push(&response([0x10, 20, 0x49, 2, 1, b'1', b'H', b'G']))
            .unwrap();
        rx.push(&response([0x21, b'B', b'H', b'4', b'1', b'J', b'X', b'M']))
            .unwrap();
        let mut short_final = response([0x22, b'N', b'1', b'0', b'9', b'1', b'8', b'6']);
        short_final.len = 7;
        assert_eq!(rx.push(&short_final), Err(Error::InvalidLength));

        rx.push(&response([0x10, 20, 0x49, 2, 1, b'1', b'H', b'G']))
            .unwrap();
        rx.push(&response([0x21, b'B', b'H', b'4', b'1', b'J', b'X', b'M']))
            .unwrap();
        let mut overlong_final = response([0x22, b'N', b'1', b'0', b'9', b'1', b'8', b'6']);
        overlong_final.len = 9;
        assert_eq!(rx.push(&overlong_final), Err(Error::InvalidLength));
    }

    #[test]
    fn scanner_state_transitions_are_consistent() {
        let mut state = ScannerState::new();
        state.rpm = 3000;
        state.mark_timeout();
        assert_eq!(state.rpm, 3000);
        assert!(state.has_all(flags::TIMEOUT | flags::STALE));
        assert!(state.has_any(flags::TIMEOUT | flags::CONNECTED));
        assert!(!state.has_all(flags::TIMEOUT | flags::CONNECTED));
        state.record_rpm(3000);
        state.record_speed(88);
        state.record_coolant(90);
        assert!(state.has_all(flags::CONNECTED));
        assert!(!state.has_any(flags::TIMEOUT | flags::STALE));
        assert_eq!(state.generation, 1);
        state.update_dtc_count(2);
        assert!(state.has_all(flags::DTC_PRESENT));
        state.update_dtc_count(0);
        assert!(!state.has_any(flags::DTC_PRESENT));
        state.set_vin(*b"1HGBH41JXMN109186");
        assert!(state.vin_valid);
    }
}
