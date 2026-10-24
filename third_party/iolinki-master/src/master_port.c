/**
 * @file master_port.c
 * @brief Per-port master state machine: init, startup handshake, cyclic ticking,
 *        frame TX/RX and OPERATE process-data exchange.
 * @ingroup iolinki_master
 *
 * Implements the single-port lifecycle of the IO-Link master: wake-up and
 * baudrate scanning during startup, transition through PREOPERATE to OPERATE,
 * scheduling of cycles and response timeouts, sending M-sequence frames and
 * decoding device responses.
 */

#include "master_internal.h"

#include "iolinki/crc.h"
#include "iolinki/frame.h"
#include "iolinki/protocol.h"

#include <string.h>

/** @brief Test-message rate order of 7.3.2.2 and Figure 36 (descending: COM3, COM2, COM1). */
static const iolink_baudrate_t g_iolink_master_baudrate_scan[] = {
    IOLINK_BAUDRATE_COM3,
    IOLINK_BAUDRATE_COM2,
    IOLINK_BAUDRATE_COM1,
};

/** @brief Verify the A.1.6 checksum of a TYPE_0 reply @c [OD, CKS].
 *
 * A TYPE_0 reply is one OD octet plus CKS (A.1.5); the checksum is the A.1.6
 * message checksum over the OD octet and CKS with bits 0-5 zeroed.
 */
static bool iolink_master_type0_reply_ok(const uint8_t* frame)
{
    const uint8_t msg[2] = {frame[0], (uint8_t) (frame[1] & 0xC0U)};

    return iolink_checksum6(msg, 2U) == (uint8_t) (frame[1] & 0x3FU);
}

/** @brief Return true if startup must read/validate device info (device-info check or inspection
 * enabled). */
static bool iolink_master_startup_validation_required(const iolink_master_config_t* config)
{
    return config->validate_device_info ||
           (config->inspection_level != IOLINK_MASTER_INSPECTION_NO_CHECK);
}

/** @brief Return the rate of the current test message, or the established rate once a
 * device answered (DL_Mode COMx, 7.3.2.2). */
static iolink_baudrate_t iolink_master_startup_baudrate(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    if (state->config.auto_baudrate) {
        return g_iolink_master_baudrate_scan[state->startup.baudrate_index];
    }

    return state->config.baudrate;
}

/** @brief Return T_BIT in nanoseconds for a baudrate (Table 9, A.3.2). */
static uint32_t iolink_master_t_bit_ns(iolink_baudrate_t baudrate)
{
    switch (baudrate) {
        case IOLINK_BAUDRATE_COM1:
            return IOLINK_MASTER_T_BIT_COM1_NS;
        case IOLINK_BAUDRATE_COM2:
            return IOLINK_MASTER_T_BIT_COM2_NS;
        case IOLINK_BAUDRATE_COM3:
        default:
            return IOLINK_MASTER_T_BIT_COM3_NS;
    }
}

/** @brief Convert a duration in bit times at @p baudrate to 100us ticks (rounded up). */
static uint32_t iolink_master_tbit_to_100us(iolink_baudrate_t baudrate, uint32_t t_bit)
{
    uint64_t ns = (uint64_t) iolink_master_t_bit_ns(baudrate) * (uint64_t) t_bit;

    return (uint32_t) ((ns + IOLINK_MASTER_NS_PER_100US - 1U) / IOLINK_MASTER_NS_PER_100US);
}

/** @brief Return the configured response timeout in 100us units.
 *
 * A.3.5/A.3.6: the deadline covers the master message (one 11 T_BIT UART frame)
 * plus the maximum device response delay (10 T_BIT). The configured value is a
 * lower bound; a value that would make the deadline zero still waits one tick.
 */
static uint32_t iolink_master_response_timeout_100us(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);
    uint32_t frame_100us =
        iolink_master_tbit_to_100us(iolink_master_startup_baudrate(port),
                                    IOLINK_MASTER_UART_FRAME_TBIT + IOLINK_MASTER_T_A_MAX_TBIT);
    uint32_t configured = state->config.response_timeout_100us;

    if (configured < frame_100us) {
        configured = frame_100us;
    }

    return (configured == 0U) ? 1U : configured;
}

/** @brief T_DMT (Table 42) in 100us ticks, measured in bit times of @p baudrate. */
static uint32_t iolink_master_t_dmt_100us(const iolink_master_port_state_t* state,
                                          iolink_baudrate_t baudrate)
{
    uint8_t t_dmt = state->config.t_dmt_tbit;

    if (t_dmt == 0U) {
        t_dmt = IOLINK_MASTER_DEFAULT_T_DMT_TBIT;
    }

    return iolink_master_tbit_to_100us(baudrate, t_dmt);
}

/** @brief Answer window for a startup test message, in 100us ticks.
 *
 * Table 46 AwaitReply_1 waits T_M-sequence for the TYPE_0 reply; A.3.6 (A.6)
 * gives at most 58 T_BIT at the rate being tried. The configured
 * response_timeout_100us widens it for adapter latency.
 */
static uint32_t iolink_master_test_message_timeout_100us(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);
    uint32_t window = iolink_master_tbit_to_100us(iolink_master_startup_baudrate(port),
                                                  IOLINK_MASTER_TYPE0_M_SEQ_MAX_TBIT);

    if (state->config.response_timeout_100us > window) {
        window = state->config.response_timeout_100us;
    }

    return (window == 0U) ? 1U : window;
}

/** @brief Set the PHY line mode via the checked config callback or, failing that, the raw PHY. */
static int iolink_master_set_mode(iolink_master_port_t* port, iolink_phy_mode_t mode)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);

    if (state->config.set_mode_checked != NULL) {
        return state->config.set_mode_checked(mode);
    }

    if ((state->phy != NULL) && (state->phy->set_mode != NULL)) {
        state->phy->set_mode(state->phy->user, mode);
    }

    return IOLINK_MASTER_STATUS_OK;
}

/** @brief Set the PHY baudrate via the checked config callback or, failing that, the raw PHY. */
static int iolink_master_set_baudrate(iolink_master_port_t* port, iolink_baudrate_t baudrate)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);

    if (state->config.set_baudrate_checked != NULL) {
        return state->config.set_baudrate_checked(baudrate);
    }

    if ((state->phy != NULL) && (state->phy->set_baudrate != NULL)) {
        state->phy->set_baudrate(state->phy->user, baudrate);
    }

    return IOLINK_MASTER_STATUS_OK;
}

/** @brief Reset the RX buffer and invoke the optional flush_rx callback to discard stale bytes. */
static int iolink_master_flush_rx(iolink_master_port_t* port)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);

    state->rx.len = 0U;
    if (state->config.flush_rx == NULL) {
        return IOLINK_MASTER_STATUS_OK;
    }

    return state->config.flush_rx();
}

/** @brief Send a full frame with prepare_tx/prepare_rx hooks, tracking send errors and error state.
 */
static bool iolink_master_send_full(iolink_master_port_t* port, const uint8_t* data, size_t len)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    const bool od_expected = ((data[0] & IOLINK_MC_RW_MASK) != 0U);
    int sent;
    int ret;

    if (state->config.prepare_tx != NULL) {
        ret = state->config.prepare_tx();
        if (ret != IOLINK_MASTER_STATUS_OK) {
            state->diagnostics.send_errors++;
            state->state = IOLINK_MASTER_STATE_ERROR;
            return false;
        }
    }

    sent = state->phy->send(state->phy->user, data, len);

    if (state->config.prepare_rx != NULL) {
        ret = state->config.prepare_rx();
        if (ret != IOLINK_MASTER_STATUS_OK) {
            state->diagnostics.send_errors++;
            state->state = IOLINK_MASTER_STATE_ERROR;
            return false;
        }
    }

    if (sent == (int) len) {
        /* Figures A.9/A.10/A.13/A.14: only the reply to a READ M-sequence (MC
           bit 7 set) carries OD octets; a WRITE reply is [PD-in] CKS. Record
           the direction of the frame now on the wire for reply framing. */
        state->rx_od_expected = od_expected;
        return true;
    }

    state->diagnostics.send_errors++;
    state->state = IOLINK_MASTER_STATE_ERROR;
    return false;
}

/** @brief Issue the wake-up request via the config callback or by sending a wake-up byte. */
static bool iolink_master_wake_up(iolink_master_port_t* port)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    int ret;

    if (state->config.wake_up == NULL) {
        state->tx_buf[0] = IOLINK_MASTER_WAKEUP_BYTE;
        return iolink_master_send_full(port, state->tx_buf, 1U);
    }

    ret = state->config.wake_up();
    if (ret == IOLINK_MASTER_STATUS_OK) {
        return true;
    }

    state->diagnostics.send_errors++;
    state->state = IOLINK_MASTER_STATE_ERROR;
    return false;
}

/** @brief Build one ISDU request frame for the current M-sequence.
 *
 * A Type-0 message is MC + CKT (+ one OD octet on a write); a Type-1/Type-2
 * OPERATE message is MC + CKT + PD-out + OD (A.2.3/A.2.4). The CKT carries the
 * M-sequence type bits and the A.1.6 checksum. Returns the frame length.
 */
static int iolink_master_build_isdu_frame(iolink_master_port_t* port, uint8_t mc,
                                          const uint8_t* od, bool multi)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    size_t pos;
    uint8_t i;

    if (!multi) {
        if (od == NULL) {
            return iolink_frame_encode_type0(mc, state->tx_buf, sizeof(state->tx_buf));
        }
        state->tx_buf[0] = mc;
        state->tx_buf[1] = 0x00U;
        for (i = 0U; i < state->od_len; i++) {
            state->tx_buf[(size_t) IOLINK_M_SEQ_HEADER_LEN + i] = od[i];
        }
        pos = (size_t) IOLINK_M_SEQ_HEADER_LEN + state->od_len;
    }
    else {
        state->tx_buf[0] = mc;
        state->tx_buf[1] = iolink_master_ckt_type_bits(state->config.m_seq_type);
        if (state->pd_out_len > 0U) {
            (void) memcpy(&state->tx_buf[IOLINK_M_SEQ_HEADER_LEN], state->pd_out,
                          state->pd_out_len);
        }
        pos = (size_t) IOLINK_M_SEQ_HEADER_LEN + state->pd_out_len;
        for (i = 0U; i < state->od_len; i++) {
            state->tx_buf[pos + i] = (od != NULL) ? od[i] : 0x00U;
        }
        pos += state->od_len;
    }

    state->tx_buf[1] = (uint8_t) (state->tx_buf[1] |
                                  iolink_checksum6(state->tx_buf, pos));
    return (int) pos;
}

/** @brief Send one M-sequence on the ISDU channel if a service is in progress.
 *
 * The ISDU octet stream (7.3.6.1, A.5) is segmented over the ISDU channel with
 * the FlowCTRL counter in the MC address bits (7.3.6.2, Table 52): requests are
 * write M-sequences carrying OD octets, response polling/reads are read
 * M-sequences. A latched transport error is signalled with FlowCTRL ABORT
 * (Table 53 T11). Returns true when a frame was emitted.
 */
static bool iolink_master_send_isdu(iolink_master_port_t* port)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    uint8_t od[IOLINK_ISDU_BUFFER_SIZE] = {0U};
    bool read = false;
    bool multi = (state->state == IOLINK_MASTER_STATE_OPERATE) &&
                 (state->config.m_seq_type != IOLINK_MASTER_M_SEQ_TYPE_0);
    uint8_t flowctrl = IOLINK_FLOWCTRL_IDLE;
    uint8_t mc;
    int frame_len;

    if (iolink_master_isdu_take_abort(port)) {
        mc = iolink_master_encode_master_command(true, IOLINK_MASTER_MC_CHANNEL_ISDU,
                                                 IOLINK_FLOWCTRL_ABORT);
        frame_len = iolink_master_build_isdu_frame(port, mc, NULL, multi);
        if (frame_len > 0) {
            (void) iolink_master_send_full(port, state->tx_buf, (size_t) frame_len);
        }
        return frame_len > 0;
    }

    if (iolink_master_isdu_take_idle(port)) {
        /* T8: one last read with FlowCTRL IDLE concludes the service. */
        mc = iolink_master_encode_master_command(true, IOLINK_MASTER_MC_CHANNEL_ISDU,
                                                 IOLINK_FLOWCTRL_IDLE);
        frame_len = iolink_master_build_isdu_frame(port, mc, NULL, multi);
        if (frame_len > 0) {
            (void) iolink_master_send_full(port, state->tx_buf, (size_t) frame_len);
        }
        return frame_len > 0;
    }

    if (!iolink_master_isdu_channel_access(port, &read, &flowctrl)) {
        return false;
    }

    mc = iolink_master_encode_master_command(read, IOLINK_MASTER_MC_CHANNEL_ISDU, flowctrl);
    if (read) {
        /* Response polling/read: the transport advances FlowCTRL without
           consuming request data. */
        (void) iolink_master_isdu_fill_od(port, od, state->od_len);
        frame_len = iolink_master_build_isdu_frame(port, mc, NULL, multi);
    }
    else {
        iolink_master_isdu_fill_od(port, od, state->od_len);
        frame_len = iolink_master_build_isdu_frame(port, mc, od, multi);
    }

    if (frame_len > 0) {
        (void) iolink_master_send_full(port, state->tx_buf, (size_t) frame_len);
    }
    return frame_len > 0;
}

/** @brief Send one M-sequence on the DIAGNOSIS channel if an event service is active.
 *
 * The event memory (7.3.8, Table 58) is read octet by octet and the readout is
 * confirmed by a write to the StatusCode at address 0 (Table 59 T8). The M-sequence
 * layout follows the configured type: TYPE_0 uses MC+CKT (write adds one OD);
 * TYPE_1/TYPE_2 use MC+CKT+PD-out+OD (A.2.3/A.2.4). Returns true when a frame
 * was emitted.
 */
static bool iolink_master_send_event(iolink_master_port_t* port)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    uint8_t mc;
    uint8_t addr = 0U;
    uint8_t od_len = 1U;
    bool read = false;
    int frame_len;

    if (!iolink_master_event_channel_access(port, &read, &addr, &od_len)) {
        return false;
    }

    if (state->config.m_seq_type == IOLINK_MASTER_M_SEQ_TYPE_0) {
        mc = iolink_master_encode_master_command(read, IOLINK_MASTER_MC_CHANNEL_DIAGNOSIS, addr);
        if (read) {
            frame_len = iolink_frame_encode_type0(mc, state->tx_buf, sizeof(state->tx_buf));
        }
        else {
            frame_len =
                iolink_frame_encode_type0_write(mc, 0x00U, state->tx_buf, sizeof(state->tx_buf));
        }
    }
    else {
        size_t od_pos;
        uint8_t i;

        mc = iolink_master_encode_master_command(read, IOLINK_MASTER_MC_CHANNEL_DIAGNOSIS, addr);
        frame_len = (int) (IOLINK_M_SEQ_HEADER_LEN + state->pd_out_len + od_len);
        state->tx_buf[0] = mc;
        state->tx_buf[1] = 0x00U;
        if (state->pd_out_len > 0U) {
            (void) memcpy(&state->tx_buf[IOLINK_M_SEQ_HEADER_LEN], state->pd_out, state->pd_out_len);
        }
        od_pos = (size_t) IOLINK_M_SEQ_HEADER_LEN + state->pd_out_len;
        for (i = 0U; i < od_len; i++) {
            state->tx_buf[od_pos + i] = 0x00U;
        }
        /* A.1.6: the CKT octet enters the checksum with its M-sequence type bits
           in place and the checksum bits zero. */
        state->tx_buf[1] = iolink_master_ckt_type_bits(state->config.m_seq_type);
        state->tx_buf[1] = (uint8_t) (state->tx_buf[1] |
                                      iolink_checksum6(state->tx_buf, (size_t) frame_len));
    }

    if (frame_len > 0) {
        if (iolink_master_send_full(port, state->tx_buf, (size_t) frame_len) && read) {
            /* Arm the OD routing only once the diagnosis read is on the wire:
             * the reply to a cyclic frame sent earlier may still land while the
             * readout is active. */
            state->event.od_expected = true;
        }
    }

    if (!read) {
        /* Table 59 T8: the StatusCode write completes the readout; the device
           release is not gated on the reply octet. */
        iolink_master_event_on_written(port);
    }

    return frame_len > 0;
}

/** @brief Return true if a new OPERATE cycle is due at the given timestamp (or pacing is inactive).
 */
static bool iolink_master_cycle_due_at(const iolink_master_port_t* port, uint32_t now_100us)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    if ((state->state != IOLINK_MASTER_STATE_OPERATE) || (state->config.min_cycle_time == 0U) ||
        !state->cycle_timer_valid) {
        return true;
    }

    return ((uint32_t) (now_100us - state->last_cycle_start_100us) >=
            (uint32_t) state->config.min_cycle_time);
}

/** @brief Return true if the current cycle overran its min cycle time at the given timestamp. */
static bool iolink_master_cycle_slipped_at(const iolink_master_port_t* port, uint32_t now_100us)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    if ((state->state != IOLINK_MASTER_STATE_OPERATE) || (state->config.min_cycle_time == 0U) ||
        !state->cycle_timer_valid) {
        return false;
    }

    return ((uint32_t) (now_100us - state->last_cycle_start_100us) >
            (uint32_t) state->config.min_cycle_time);
}

/** @brief Return the cycle jitter (100us units elapsed beyond the min cycle time) at the timestamp.
 */
static uint32_t iolink_master_cycle_jitter_at(const iolink_master_port_t* port, uint32_t now_100us)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);
    uint32_t elapsed;

    if ((state->state != IOLINK_MASTER_STATE_OPERATE) || (state->config.min_cycle_time == 0U) ||
        !state->cycle_timer_valid) {
        return 0U;
    }

    elapsed = (uint32_t) (now_100us - state->last_cycle_start_100us);
    if (elapsed <= (uint32_t) state->config.min_cycle_time) {
        return 0U;
    }

    return (uint32_t) (elapsed - (uint32_t) state->config.min_cycle_time);
}

int iolink_master_get_next_tick_time(const iolink_master_port_t* port, uint32_t now_100us,
                                     uint32_t* out_next_100us)
{
    const iolink_master_port_state_t* state;
    uint32_t cycle_due;

    if ((port == NULL) || (out_next_100us == NULL)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    if ((state->state == IOLINK_MASTER_STATE_INACTIVE) ||
        (state->state == IOLINK_MASTER_STATE_ERROR)) {
        *out_next_100us = now_100us;
        return IOLINK_MASTER_STATUS_OK;
    }

    if (state->awaiting_response) {
        *out_next_100us = (now_100us >= state->response_deadline_100us)
                              ? now_100us
                              : state->response_deadline_100us;
        return IOLINK_MASTER_STATUS_OK;
    }

    if ((state->state == IOLINK_MASTER_STATE_STARTUP) && state->send_ready_valid &&
        (now_100us < state->send_ready_at_100us)) {
        /* Waiting T_REN + T_DMT, T_DMT, T_DWU or T_SD before the next startup transmission. */
        *out_next_100us = state->send_ready_at_100us;
        return IOLINK_MASTER_STATUS_OK;
    }

    if ((state->state != IOLINK_MASTER_STATE_OPERATE) || (state->config.min_cycle_time == 0U) ||
        !state->cycle_timer_valid) {
        *out_next_100us = now_100us;
        return IOLINK_MASTER_STATUS_OK;
    }

    cycle_due =
        (uint32_t) (state->last_cycle_start_100us + (uint32_t) state->config.min_cycle_time);
    *out_next_100us = (now_100us >= cycle_due) ? now_100us : cycle_due;
    return IOLINK_MASTER_STATUS_OK;
}

/** @brief Shared tick engine: polls RX, handles response timeouts, and paces/runs OPERATE cycles.
 */
static int iolink_master_tick_common(iolink_master_port_t* port, iolink_master_tick_event_t event,
                                     bool pace_cycles, uint32_t now_100us)
{
    iolink_master_port_state_t* state;
    uint32_t cycle_count_before;
    int rx_ret;
    int timeout_ret;

    if ((port == NULL) || (event > IOLINK_MASTER_TICK_RESPONSE_TIMEOUT)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    rx_ret = iolink_master_poll_rx(port);
    if (rx_ret < 0) {
        return rx_ret;
    }

    state = iolink_master_port_state(port);
    if (event == IOLINK_MASTER_TICK_RESPONSE_TIMEOUT) {
        if (pace_cycles && !state->awaiting_response) {
            return rx_ret;
        }

        if (state->awaiting_response) {
            /* Drop a late or partial reply from the timed-out request so its
               bytes cannot be framed as the reply to the next one. */
            (void) iolink_master_flush_rx(port);
        }
        state->awaiting_response = false;
        timeout_ret = iolink_master_on_timeout_at(port, now_100us, pace_cycles);
        if (timeout_ret != IOLINK_MASTER_STATUS_OK) {
            return timeout_ret;
        }
    }

    if (event != IOLINK_MASTER_TICK_CYCLE_DUE) {
        return rx_ret;
    }

    /* One M-sequence at a time (A.1.2): do not put a new request on the wire
       while a reply is still outstanding. Replies carry direction-dependent
       lengths (A.9/A.10/A.13/A.14) and the ISDU transport advances on reply
       receipt, so a pipelined send would mis-frame or desynchronize the
       stream. The response deadline (or the reply itself) releases the port;
       the caller retries on a later tick. */
    if (state->awaiting_response) {
        return rx_ret;
    }

    if (pace_cycles && !iolink_master_cycle_due_at(port, now_100us)) {
        return rx_ret;
    }

    cycle_count_before = state->cycle_count;
    iolink_master_process_at(port, now_100us, pace_cycles);

    /* iolink_master_process() increments cycle_count through the port pointer on a
       successful operate send; cppcheck does not model that side effect. */
    /* cppcheck-suppress knownConditionTrueFalse */
    if (pace_cycles && (state->cycle_count != cycle_count_before)) {
        uint32_t jitter_100us = iolink_master_cycle_jitter_at(port, now_100us);

        if (iolink_master_cycle_slipped_at(port, now_100us)) {
            state->diagnostics.cycle_slips++;
        }
        state->diagnostics.last_cycle_jitter_100us = jitter_100us;
        if (jitter_100us > state->diagnostics.max_cycle_jitter_100us) {
            state->diagnostics.max_cycle_jitter_100us = jitter_100us;
        }

        state->last_cycle_start_100us = now_100us;
        state->response_deadline_100us =
            (uint32_t) (now_100us + (uint32_t) iolink_master_response_timeout_100us(port));
        state->cycle_timer_valid = true;
        state->awaiting_response = true;
    }

    return rx_ret;
}

int iolink_master_init(iolink_master_port_t* port, const iolink_phy_api_t* phy,
                       const iolink_master_config_t* config)
{
    int ret;

    if ((port == NULL) || (phy == NULL) || (config == NULL) ||
        (config->pd_in_len > IOLINK_PD_IN_MAX_SIZE) ||
        (config->pd_out_len > IOLINK_PD_OUT_MAX_SIZE) ||
        (config->m_seq_type > IOLINK_MASTER_M_SEQ_TYPE_2_V) ||
        (config->baudrate > IOLINK_BAUDRATE_COM3) ||
        (config->port_mode > IOLINK_MASTER_PORT_MODE_DEACTIVATED) ||
        ((config->m_seq_type == IOLINK_MASTER_M_SEQ_TYPE_0) &&
         ((config->pd_in_len > 0U) || (config->pd_out_len > 0U)))) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    (void) memset(port, 0, sizeof(*port));
    iolink_master_port_state(port)->phy = phy;
    iolink_master_port_state(port)->config = *config;
    iolink_master_port_state(port)->od_len = iolink_master_od_len_for_type(config->m_seq_type);
    iolink_master_port_state(port)->pd_in_len = config->pd_in_len;
    iolink_master_port_state(port)->pd_out_len = config->pd_out_len;
    /* No DeviceStatus has been read yet; the failure sentinel matches the NULL-port contract. */
    iolink_master_port_state(port)->diagnostics.device_status = IOLINK_DEVICE_STATUS_FAILURE;
    iolink_master_port_state(port)->startup.baudrate_index = 0U;
    iolink_master_port_state(port)->state = (config->port_mode == IOLINK_MASTER_PORT_MODE_IOLINK)
                                                ? IOLINK_MASTER_STATE_STARTUP
                                                : IOLINK_MASTER_STATE_INACTIVE;

    if (phy->init != NULL) {
        ret = phy->init(phy->user);
        if (ret != 0) {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return ret;
        }
    }

    if (config->port_mode == IOLINK_MASTER_PORT_MODE_DEACTIVATED) {
        ret = iolink_master_set_mode(port, IOLINK_PHY_MODE_INACTIVE);
        if (ret != IOLINK_MASTER_STATUS_OK) {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return ret;
        }
        return IOLINK_MASTER_STATUS_OK;
    }

    if ((config->port_mode == IOLINK_MASTER_PORT_MODE_DI) ||
        (config->port_mode == IOLINK_MASTER_PORT_MODE_DQ)) {
        ret = iolink_master_set_mode(port, IOLINK_PHY_MODE_SIO);
        if (ret != IOLINK_MASTER_STATUS_OK) {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return ret;
        }
        return IOLINK_MASTER_STATUS_OK;
    }

    ret = iolink_master_flush_rx(port);
    if (ret != IOLINK_MASTER_STATUS_OK) {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }

    ret = iolink_master_set_baudrate(port, iolink_master_startup_baudrate(port));
    if (ret != IOLINK_MASTER_STATUS_OK) {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }

    ret = iolink_master_set_mode(port, IOLINK_PHY_MODE_SDCI);
    if (ret != IOLINK_MASTER_STATUS_OK) {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }

    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_validate_phy_contract(const iolink_phy_api_t* phy,
                                        const iolink_master_config_t* config)
{
    if ((phy == NULL) || (config == NULL) ||
        (config->port_mode > IOLINK_MASTER_PORT_MODE_DEACTIVATED)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    switch (config->port_mode) {
        case IOLINK_MASTER_PORT_MODE_IOLINK:
            if ((phy->send == NULL) || (phy->recv_byte == NULL) ||
                (config->set_mode_checked == NULL) || (config->set_baudrate_checked == NULL) ||
                (config->flush_rx == NULL) || (config->prepare_tx == NULL) ||
                (config->prepare_rx == NULL) || (config->wake_up == NULL)) {
                return IOLINK_MASTER_ERR_UNSUPPORTED_PHY;
            }
            return IOLINK_MASTER_STATUS_OK;
        case IOLINK_MASTER_PORT_MODE_DI:
            if ((config->set_mode_checked == NULL) || (config->read_cq_line_checked == NULL)) {
                return IOLINK_MASTER_ERR_UNSUPPORTED_PHY;
            }
            return IOLINK_MASTER_STATUS_OK;
        case IOLINK_MASTER_PORT_MODE_DQ:
            if ((config->set_mode_checked == NULL) || (phy->set_cq_line == NULL)) {
                return IOLINK_MASTER_ERR_UNSUPPORTED_PHY;
            }
            return IOLINK_MASTER_STATUS_OK;
        case IOLINK_MASTER_PORT_MODE_DEACTIVATED:
            if (config->set_mode_checked == NULL) {
                return IOLINK_MASTER_ERR_UNSUPPORTED_PHY;
            }
            return IOLINK_MASTER_STATUS_OK;
        default:
            return IOLINK_MASTER_ERR_INVALID_ARG;
    }
}

int iolink_master_restart(iolink_master_port_t* port)
{
    const iolink_phy_api_t* phy;
    iolink_master_config_t config;

    if ((port == NULL) || (iolink_master_port_state(port)->phy == NULL)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    phy = iolink_master_port_state(port)->phy;
    config = iolink_master_port_state(port)->config;

    return iolink_master_init(port, phy, &config);
}

int iolink_master_on_timeout(iolink_master_port_t* port)
{
    return iolink_master_on_timeout_at(port, 0U, false);
}

/** @brief Effective n_WU (Table 42): configured value or the spec value 2. */
static uint8_t iolink_master_wake_retry_limit(const iolink_master_port_state_t* state)
{
    return (state->config.wake_retry_limit == 0U) ? IOLINK_MASTER_DEFAULT_WAKE_RETRY_LIMIT
                                                  : state->config.wake_retry_limit;
}

/** @brief Effective T_DWU wake-retry delay in 100us units (Table 42). */
static uint32_t iolink_master_t_dwu_100us(const iolink_master_port_state_t* state)
{
    return (state->config.t_dwu_100us == 0U) ? IOLINK_MASTER_DEFAULT_T_DWU_100US
                                             : state->config.t_dwu_100us;
}

/** @brief Effective T_SD wake-up sequence spacing in 100us units (Table 42). */
static uint32_t iolink_master_t_sd_100us(const iolink_master_port_state_t* state)
{
    return (state->config.t_sd_100us == 0U) ? IOLINK_MASTER_DEFAULT_T_SD_100US
                                            : state->config.t_sd_100us;
}

/** @brief Hold the next startup transmission until @p delay_100us after @p now_100us.
 *
 * Only the timed entry points have a clock; the untimed ones leave the gate open.
 */
static void iolink_master_arm_send_gate(iolink_master_port_t* port, uint32_t now_100us,
                                        uint32_t delay_100us, bool timed)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);

    if (timed) {
        state->send_ready_at_100us = (uint32_t) (now_100us + delay_100us);
        state->send_ready_valid = true;
    }
}

/** @brief Point the rate scan back at its first rate (COM3 under auto-baud, Figure 36 T15). */
static int iolink_master_reset_rate_scan(iolink_master_port_t* port)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);

    if (!state->config.auto_baudrate || (state->startup.baudrate_index == 0U)) {
        return IOLINK_MASTER_STATUS_OK;
    }

    state->startup.baudrate_index = 0U;
    return iolink_master_set_baudrate(port, iolink_master_startup_baudrate(port));
}

/**
 * @brief Restart communication from the wake-up after COMLOST.
 *
 * Table 46: once the message handler runs out of retries it returns to
 * Inactive_0 and reports MHInfo(COMLOST); the DL-mode handler (Table 44 T9/T14)
 * and System Management (Table 85 T3) then start a new wake-up procedure. The
 * port re-enters STARTUP with a fresh Retry counter instead of latching ERROR;
 * ERROR stays reserved for PHY failures. The wake-up waits T_DWU when a clock
 * is available.
 */
static int iolink_master_restart_establish(iolink_master_port_t* port, uint32_t now_100us,
                                           bool timed)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    int ret;

    state->state = IOLINK_MASTER_STATE_STARTUP;
    state->startup.step = IOLINK_MASTER_STARTUP_STEP_WAKE;
    state->startup.wake_attempts = 0U;
    state->diagnostics.rx_retry_count = 0U;
    state->awaiting_response = false;
    ret = iolink_master_reset_rate_scan(port);
    if (ret != IOLINK_MASTER_STATUS_OK) {
        state->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }
    iolink_master_arm_send_gate(port, now_100us, iolink_master_t_dwu_100us(state), timed);
    return IOLINK_MASTER_STATUS_PENDING;
}

/**
 * @brief Advance the establish-communication submachine after a test message failed.
 *
 * Figure 36 / Table 44: no answer (or no decodable answer, Table 46 T3/T4) at
 * COM3 or COM2 moves to the next lower rate after T_DMT (T16, T17). No answer at
 * the last rate increments Retry (T18); while Retry < n_WU + 1 a new wake-up
 * request follows after T_DWU (T19, Figure 32). Otherwise the sequence failed
 * (T5): it is reported, the PHY goes inactive (7.3.2.2) and the next sequence
 * waits T_SD (Figure 33).
 */
static int iolink_master_establish_attempt_failed(iolink_master_port_t* port, uint32_t now_100us,
                                                  bool timed)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    const uint8_t last_rate = (uint8_t) ((sizeof(g_iolink_master_baudrate_scan) /
                                          sizeof(g_iolink_master_baudrate_scan[0])) -
                                         1U);
    int ret;

    state->awaiting_response = false;

    if (state->config.auto_baudrate && (state->startup.baudrate_index < last_rate)) {
        state->startup.baudrate_index++;
        ret = iolink_master_set_baudrate(port, iolink_master_startup_baudrate(port));
        if (ret != IOLINK_MASTER_STATUS_OK) {
            state->state = IOLINK_MASTER_STATE_ERROR;
            return ret;
        }
        state->startup.step = IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0;
        iolink_master_arm_send_gate(
            port, now_100us, iolink_master_t_dmt_100us(state, iolink_master_startup_baudrate(port)),
            timed);
        return IOLINK_MASTER_STATUS_PENDING;
    }

    state->startup.wake_attempts++;
    ret = iolink_master_reset_rate_scan(port);
    if (ret != IOLINK_MASTER_STATUS_OK) {
        state->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }

    if (state->startup.wake_attempts <= iolink_master_wake_retry_limit(state)) {
        state->startup.step = IOLINK_MASTER_STARTUP_STEP_WAKE;
        iolink_master_arm_send_gate(port, now_100us, iolink_master_t_dwu_100us(state), timed);
        return IOLINK_MASTER_STATUS_PENDING;
    }

    state->startup.wake_attempts = 0U;
    state->diagnostics.establish_failures++;
    ret = iolink_master_set_mode(port, IOLINK_PHY_MODE_INACTIVE);
    if (ret != IOLINK_MASTER_STATUS_OK) {
        state->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }
    state->startup.step = IOLINK_MASTER_STARTUP_STEP_INACTIVE;
    iolink_master_arm_send_gate(port, now_100us, iolink_master_t_sd_100us(state), timed);
    return IOLINK_MASTER_ERR_RETRY_LIMIT;
}

int iolink_master_on_timeout_at(iolink_master_port_t* port, uint32_t now_100us, bool timed)
{
    int ret;

    if (port == NULL) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    ret = iolink_master_flush_rx(port);
    if (ret != IOLINK_MASTER_STATUS_OK) {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }

    if (iolink_master_port_state(port)->state != IOLINK_MASTER_STATE_STARTUP) {
        if (iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_OPERATE) {
            iolink_master_port_state(port)->diagnostics.response_timeouts++;
            if (iolink_master_port_state(port)->diagnostics.rx_retry_count <
                IOLINK_MASTER_RX_RETRY_LIMIT) {
                iolink_master_port_state(port)->diagnostics.rx_retry_count++;
                return IOLINK_MASTER_STATUS_PENDING;
            }

            return iolink_master_restart_establish(port, now_100us, timed);
        }

        return IOLINK_MASTER_STATUS_OK;
    }

    if (iolink_master_port_state(port)->phy == NULL) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if (iolink_master_port_state(port)->startup.step == IOLINK_MASTER_STARTUP_STEP_INACTIVE) {
        /* Nothing is in flight while the port waits T_SD. */
        return IOLINK_MASTER_STATUS_PENDING;
    }

    return iolink_master_establish_attempt_failed(port, now_100us, timed);
}

int iolink_master_tick(iolink_master_port_t* port, bool response_timeout)
{
    return iolink_master_tick_event(port, response_timeout ? IOLINK_MASTER_TICK_RESPONSE_TIMEOUT
                                                           : IOLINK_MASTER_TICK_CYCLE_DUE);
}

int iolink_master_tick_event(iolink_master_port_t* port, iolink_master_tick_event_t event)
{
    return iolink_master_tick_common(port, event, false, 0U);
}

int iolink_master_tick_at(iolink_master_port_t* port, iolink_master_tick_event_t event,
                          uint32_t now_100us)
{
    return iolink_master_tick_common(port, event, true, now_100us);
}

void iolink_master_process(iolink_master_port_t* port)
{
    iolink_master_process_at(port, 0U, false);
}

void iolink_master_process_at(iolink_master_port_t* port, uint32_t now_100us, bool timed)
{
    int frame_len;
    size_t od_pos;

    if ((port == NULL) || (iolink_master_port_state(port)->phy == NULL) ||
        (iolink_master_port_state(port)->phy->send == NULL)) {
        return;
    }

    if (timed && iolink_master_port_state(port)->send_ready_valid &&
        (now_100us < iolink_master_port_state(port)->send_ready_at_100us)) {
        /* Table 42 / Table 10: hold off until T_REN + T_DMT, T_DMT, T_DWU or T_SD elapsed. */
        return;
    }

    if (iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_STARTUP) {
        if (iolink_master_port_state(port)->startup.step == IOLINK_MASTER_STARTUP_STEP_INACTIVE) {
            /* T_SD elapsed after a failed wake-up retry sequence (Figure 33): put
               the PHY back into SDCI and start the next sequence. */
            if (iolink_master_set_mode(port, IOLINK_PHY_MODE_SDCI) != IOLINK_MASTER_STATUS_OK) {
                iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
                return;
            }
            iolink_master_port_state(port)->startup.step = IOLINK_MASTER_STARTUP_STEP_WAKE;
        }

        if (iolink_master_port_state(port)->startup.step == IOLINK_MASTER_STARTUP_STEP_WAKE) {
            /* Figure 36 WURQ_5: PL_WakeUp, then the first test message no earlier
               than T_REN (Table 10) plus T_DMT at its rate (7.3.2.2, Table 42). */
            if (iolink_master_wake_up(port)) {
                iolink_master_port_state(port)->startup.step =
                    IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0;
                iolink_master_arm_send_gate(
                    port, now_100us,
                    IOLINK_MASTER_T_REN_MAX_100US +
                        iolink_master_t_dmt_100us(iolink_master_port_state(port),
                                                  iolink_master_startup_baudrate(port)),
                    timed);
            }
            return;
        }

        if (iolink_master_port_state(port)->startup.step == IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0) {
            /* Table 46 T1: the test message is a TYPE_0 READ of the Direct
               Parameter page MinCycleTime octet (MC = 0xA2) on the page
               communication channel. */
            frame_len = iolink_frame_encode_type0(
                iolink_master_encode_master_command(true, IOLINK_MASTER_MC_CHANNEL_PAGE,
                                                    IOLINK_MASTER_DPP1_OFF_MIN_CYCLE_TIME),
                iolink_master_port_state(port)->tx_buf,
                sizeof(iolink_master_port_state(port)->tx_buf));
            if (frame_len > 0) {
                if (iolink_master_send_full(port, iolink_master_port_state(port)->tx_buf,
                                            (size_t) frame_len)) {
                    iolink_master_port_state(port)->startup.step =
                        IOLINK_MASTER_STARTUP_STEP_AWAIT_RESPONSE;
                    if (timed) {
                        iolink_master_port_state(port)->response_deadline_100us =
                            (uint32_t) (now_100us + iolink_master_test_message_timeout_100us(port));
                        iolink_master_port_state(port)->awaiting_response = true;
                    }
                }
            }
            return;
        }
    }

    if (iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_PREOPERATE) {
        int ret;

        if (iolink_master_send_event(port)) {
            return;
        }

        if (iolink_master_send_isdu(port)) {
            return;
        }

        if (iolink_master_startup_validation_required(&iolink_master_port_state(port)->config) &&
            !iolink_master_port_state(port)->device_info.valid) {
            ret = iolink_master_read_device_info(port);
            if (ret == IOLINK_MASTER_STATUS_PENDING) {
                return;
            }
            if (ret < 0) {
                iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
                return;
            }
        }

        if (iolink_master_startup_validation_required(&iolink_master_port_state(port)->config) &&
            (iolink_master_validate_device_info(port) != 0)) {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return;
        }

        /* Spec transition to OPERATE: write MasterCommand DeviceOperate (0x99,
           Table B.2) to Direct Parameter page address 0x00 on the page channel,
           i.e. a Type-0 WRITE frame (MC = 0x20, one OD data octet 0x99). */
        frame_len = iolink_frame_encode_type0_write(
            iolink_master_encode_master_command(false, IOLINK_MASTER_MC_CHANNEL_PAGE,
                                                IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND),
            IOLINK_CMD_DEVICE_OPERATE, iolink_master_port_state(port)->tx_buf,
            sizeof(iolink_master_port_state(port)->tx_buf));
        if (frame_len > 0) {
            if (iolink_master_send_full(port, iolink_master_port_state(port)->tx_buf,
                                        (size_t) frame_len)) {
                /* Figure A.5: a Type-0 WRITE is answered by the CKS octet alone.
                   Consume and verify it before the first cyclic exchange, or it
                   would prefix the next reply and corrupt every frame after it. */
                iolink_master_port_state(port)->startup.step =
                    IOLINK_MASTER_STARTUP_STEP_AWAIT_OPERATE_ACK;
            }
        }
        return;
    }

    if (iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_OPERATE) {
        /* An event readout on the DIAGNOSIS channel (7.3.8.3) preempts the
           cyclic exchange; an active ISDU service then takes the cycle. */
        if (iolink_master_send_event(port)) {
            return;
        }

        if (iolink_master_send_isdu(port)) {
            return;
        }

        if ((iolink_master_port_state(port)->config.m_seq_type == IOLINK_MASTER_M_SEQ_TYPE_0) &&
            (iolink_master_port_state(port)->config.pd_in_len == 0U) &&
            (iolink_master_port_state(port)->pd_out_len == 0U)) {
            uint8_t od = 0U;
            iolink_master_isdu_fill_od(port, &od, 1U);
            frame_len = iolink_frame_encode_type0(od, iolink_master_port_state(port)->tx_buf,
                                                  sizeof(iolink_master_port_state(port)->tx_buf));
            if (frame_len > 0) {
                if (iolink_master_send_full(port, iolink_master_port_state(port)->tx_buf,
                                            (size_t) frame_len)) {
                    iolink_master_port_state(port)->cycle_count++;
                }
            }
            return;
        }

        /* A.2.3/A.2.4: a TYPE_1/TYPE_2 master message is MC, CKT, PD-out, OD; the
           CKT carries the M-sequence type in bits 6-7 and the A.1.6 checksum in
           bits 0-5 (there is no trailing checksum octet in the request). */
        frame_len = (int) (IOLINK_M_SEQ_HEADER_LEN + iolink_master_port_state(port)->pd_out_len +
                           iolink_master_port_state(port)->od_len);
        iolink_master_port_state(port)->tx_buf[0] =
            iolink_master_encode_master_command(false, IOLINK_MASTER_MC_CHANNEL_PROCESS, 0U);
        iolink_master_port_state(port)->tx_buf[1] = 0x00U;
        od_pos = (size_t) IOLINK_M_SEQ_HEADER_LEN + iolink_master_port_state(port)->pd_out_len;
        if (iolink_master_port_state(port)->pd_out_len > 0U) {
            (void) memcpy(&iolink_master_port_state(port)->tx_buf[IOLINK_M_SEQ_HEADER_LEN],
                          iolink_master_port_state(port)->pd_out,
                          iolink_master_port_state(port)->pd_out_len);
        }
        iolink_master_isdu_fill_od(port, &iolink_master_port_state(port)->tx_buf[od_pos],
                                   iolink_master_port_state(port)->od_len);
        /* A.1.6: type bits in place, checksum bits zero, then fold the checksum in. */
        iolink_master_port_state(port)->tx_buf[1] =
            iolink_master_ckt_type_bits(iolink_master_port_state(port)->config.m_seq_type);
        iolink_master_port_state(port)->tx_buf[1] =
            (uint8_t) (iolink_master_port_state(port)->tx_buf[1] |
                       iolink_checksum6(iolink_master_port_state(port)->tx_buf, (size_t) frame_len));

        if (iolink_master_send_full(port, iolink_master_port_state(port)->tx_buf,
                                    (size_t) frame_len)) {
            iolink_master_port_state(port)->cycle_count++;
        }
    }
}

int iolink_master_poll_rx(iolink_master_port_t* port)
{
    uint8_t byte;
    uint8_t expected_len;
    int recv_ret;
    int frame_ret;
    int frames = 0;

    if ((port == NULL) || (iolink_master_port_state(port)->phy == NULL)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if (iolink_master_port_state(port)->phy->recv_byte == NULL) {
        return IOLINK_MASTER_STATUS_OK;
    }

    if ((iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_STARTUP) &&
        (iolink_master_port_state(port)->startup.step ==
         IOLINK_MASTER_STARTUP_STEP_AWAIT_RESPONSE)) {
        expected_len = IOLINK_M_SEQ_TYPE0_LEN;
    }
    else if (iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_PREOPERATE) {
        expected_len = (iolink_master_port_state(port)->startup.step ==
                        IOLINK_MASTER_STARTUP_STEP_AWAIT_OPERATE_ACK)
                           ? 1U
                           : IOLINK_M_SEQ_TYPE0_LEN;
    }
    else if (iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_OPERATE) {
        /* Figures A.9/A.10/A.13/A.14: only the reply to a READ M-sequence
           carries OD octets; a WRITE reply is [PD-in] CKS. A TYPE_0 OPERATE
           port (no PD) keeps its established two-octet expectation: its
           replies are framed by the TYPE_0 branch of iolink_master_on_rx. */
        const iolink_master_port_state_t* state = iolink_master_port_const_state(port);
        const bool type0_operate = (state->config.m_seq_type == IOLINK_MASTER_M_SEQ_TYPE_0) &&
                                   (state->config.pd_in_len == 0U) && (state->pd_out_len == 0U);
        const uint8_t od_octets = (type0_operate || state->rx_od_expected) ? state->od_len : 0U;
        expected_len = (uint8_t) (state->config.pd_in_len + od_octets + 1U);
    }
    else {
        return IOLINK_MASTER_STATUS_OK;
    }

    for (;;) {
        recv_ret = iolink_master_port_state(port)->phy->recv_byte(
            iolink_master_port_state(port)->phy->user, &byte);
        if (recv_ret <= 0) {
            break;
        }

        if (iolink_master_port_state(port)->rx.len >=
            sizeof(iolink_master_port_state(port)->rx.buf)) {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            iolink_master_port_state(port)->rx.len = 0U;
            return IOLINK_MASTER_ERR_CHECKSUM;
        }

        iolink_master_port_state(port)->rx.buf[iolink_master_port_state(port)->rx.len++] = byte;

        if (iolink_master_port_state(port)->rx.len >= expected_len) {
            frame_ret = iolink_master_on_rx(port, iolink_master_port_state(port)->rx.buf,
                                            iolink_master_port_state(port)->rx.len);
            iolink_master_port_state(port)->rx.len = 0U;

            if (frame_ret != 0) {
                return frame_ret;
            }

            frames++;
        }
    }

    if (recv_ret < 0) {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        iolink_master_port_state(port)->rx.len = 0U;
        return IOLINK_MASTER_ERR_FRAME;
    }

    return frames;
}

/** @brief Route received OD octets to the active acyclic service.
 *
 * The DIAGNOSIS event readout (7.3.8) and the ISDU transport (7.3.6) are
 * mutually exclusive services; whichever is active consumes the reply OD. While
 * the event readout is active only the reply to an actually sent diagnosis read
 * is routed into it: a stray cyclic reply in flight when the readout starts must
 * not be captured as the event memory StatusCode.
 */
static void iolink_master_route_od(iolink_master_port_t* port, const uint8_t* od, uint8_t od_len)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    bool read = false;
    uint8_t addr = 0U;
    uint8_t event_od_len = 1U;

    if (iolink_master_event_channel_access(port, &read, &addr, &event_od_len)) {
        if (read && state->event.od_expected) {
            state->event.od_expected = false;
            iolink_master_event_on_od(port, od, od_len);
        }
        return;
    }

    iolink_master_isdu_on_od(port, od, od_len);
}

int iolink_master_on_rx(iolink_master_port_t* port, const uint8_t* data, uint8_t len)
{
    iolink_frame_operate_response_t resp;

    if ((port == NULL) || (data == NULL) || (len == 0U)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if (iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_STARTUP) {
        if (iolink_master_port_state(port)->startup.step !=
            IOLINK_MASTER_STARTUP_STEP_AWAIT_RESPONSE) {
            /* No test message is in flight: nothing can answer it. */
            return IOLINK_MASTER_ERR_FRAME;
        }

        if (len != IOLINK_M_SEQ_TYPE0_LEN) {
            return IOLINK_MASTER_ERR_FRAME;
        }

        if (!iolink_master_type0_reply_ok(data)) {
            /* Table 46 AwaitReply_1/T4: an undecodable answer to a test message
               is no answer. There is no message retry during establish; the
               attempt ends at its deadline and the scan moves on (Figure 36). */
            iolink_master_port_state(port)->diagnostics.checksum_errors++;
            return IOLINK_MASTER_ERR_CHECKSUM;
        }

        iolink_master_port_state(port)->diagnostics.rx_retry_count = 0U;
        iolink_master_port_state(port)->startup.wake_attempts = 0U;
        iolink_master_port_state(port)->awaiting_response = false;
        /* Table B.3: the startup probe reads MinCycleTime; keep it for every
           inspection level, not only when device-info validation is enabled. */
        iolink_master_port_state(port)->device_info.min_cycle_time = data[0];
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_PREOPERATE;
        return IOLINK_MASTER_STATUS_OK;
    }

    if ((iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_PREOPERATE) &&
        (iolink_master_port_state(port)->startup.step ==
         IOLINK_MASTER_STARTUP_STEP_AWAIT_OPERATE_ACK)) {
        /* CKS-only reply to the DeviceOperate write (Figure A.5, A.1.5): the
           checksum covers the CKS octet with bits 0-5 zero. */
        const uint8_t flags = (uint8_t) (data[0] & 0xC0U);

        if (len != 1U) {
            return IOLINK_MASTER_ERR_FRAME;
        }
        if (iolink_checksum6(&flags, 1U) != (uint8_t) (data[0] & 0x3FU)) {
            iolink_master_port_state(port)->diagnostics.checksum_errors++;
            if (iolink_master_port_state(port)->diagnostics.rx_retry_count <
                IOLINK_MASTER_RX_RETRY_LIMIT) {
                iolink_master_port_state(port)->diagnostics.rx_retry_count++;
            }
            else {
                /* MaxRetry exhausted (Table 46 ErrorHandling): COMLOST. */
                (void) iolink_master_restart_establish(port, 0U, false);
            }
            return IOLINK_MASTER_ERR_CHECKSUM;
        }
        iolink_master_port_state(port)->diagnostics.rx_retry_count = 0U;
        iolink_master_port_state(port)->awaiting_response = false;
        iolink_master_port_state(port)->diagnostics.event_pending = ((data[0] & 0x80U) != 0U);
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_OPERATE;
        return IOLINK_MASTER_STATUS_OK;
    }

    if (iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_PREOPERATE) {
        if (len != IOLINK_M_SEQ_TYPE0_LEN) {
            return IOLINK_MASTER_ERR_FRAME;
        }

        if (!iolink_master_type0_reply_ok(data)) {
            iolink_master_port_state(port)->diagnostics.checksum_errors++;
            if (iolink_master_port_state(port)->diagnostics.rx_retry_count <
                IOLINK_MASTER_RX_RETRY_LIMIT) {
                iolink_master_port_state(port)->diagnostics.rx_retry_count++;
            }
            else {
                /* MaxRetry exhausted (Table 46 ErrorHandling): COMLOST. */
                (void) iolink_master_restart_establish(port, 0U, false);
            }
            return IOLINK_MASTER_ERR_CHECKSUM;
        }

        iolink_master_port_state(port)->diagnostics.rx_retry_count = 0U;
        iolink_master_port_state(port)->awaiting_response = false;
        iolink_master_route_od(port, data, 1U);
        return IOLINK_MASTER_STATUS_OK;
    }

    if ((iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_OPERATE) &&
        (iolink_master_port_state(port)->config.m_seq_type == IOLINK_MASTER_M_SEQ_TYPE_0) &&
        (iolink_master_port_state(port)->config.pd_in_len == 0U) &&
        (iolink_master_port_state(port)->pd_out_len == 0U)) {
        if (len != IOLINK_M_SEQ_TYPE0_LEN) {
            return IOLINK_MASTER_ERR_FRAME;
        }

        if (!iolink_master_type0_reply_ok(data)) {
            iolink_master_port_state(port)->diagnostics.checksum_errors++;
            if (iolink_master_port_state(port)->diagnostics.rx_retry_count <
                IOLINK_MASTER_RX_RETRY_LIMIT) {
                iolink_master_port_state(port)->diagnostics.rx_retry_count++;
            }
            else {
                /* MaxRetry exhausted (Table 46 ErrorHandling): COMLOST. */
                (void) iolink_master_restart_establish(port, 0U, false);
            }
            return IOLINK_MASTER_ERR_CHECKSUM;
        }

        iolink_master_port_state(port)->diagnostics.rx_retry_count = 0U;
        iolink_master_port_state(port)->awaiting_response = false;
        iolink_master_route_od(port, data, 1U);
        return IOLINK_MASTER_STATUS_OK;
    }

    /* Figures A.9/A.10/A.13/A.14: the reply to a READ M-sequence is
       [OD][PD-in] CKS; a WRITE reply carries no OD ([PD-in] CKS). */
    if (iolink_frame_decode_operate_response(
            data, len, iolink_master_port_state(port)->config.pd_in_len,
            iolink_master_port_state(port)->rx_od_expected
                ? iolink_master_port_state(port)->od_len
                : 0U,
            &resp) != 0) {
        return IOLINK_MASTER_ERR_FRAME;
    }

    if (!resp.checksum_ok) {
        iolink_master_port_state(port)->diagnostics.checksum_errors++;
        if (iolink_master_port_state(port)->diagnostics.rx_retry_count <
            IOLINK_MASTER_RX_RETRY_LIMIT) {
            iolink_master_port_state(port)->diagnostics.rx_retry_count++;
        }
        else {
            /* MaxRetry exhausted (Table 46 ErrorHandling_17): COMLOST. */
            (void) iolink_master_restart_establish(port, 0U, false);
        }
        return IOLINK_MASTER_ERR_CHECKSUM;
    }

    iolink_master_port_state(port)->diagnostics.rx_retry_count = 0U;
    iolink_master_port_state(port)->awaiting_response = false;
    iolink_master_port_state(port)->diagnostics.od_status = resp.status;

    /*
     * Dispatch on the rising edge of the OD Event flag. This turns event
     * handling from "poll diagnostics.event_pending yourself" into a
     * notification: the application reacts by reading event details (which then
     * dispatches each decoded event through config.event_handler).
     */
    if (resp.event_pending && !iolink_master_port_state(port)->diagnostics.event_pending &&
        (iolink_master_port_state(port)->config.event_pending_handler != NULL)) {
        iolink_master_port_state(port)->config.event_pending_handler(
            iolink_master_port_state(port)->config.event_user);
    }
    iolink_master_port_state(port)->diagnostics.event_pending = resp.event_pending;

    if (resp.pd_valid) {
        (void) memcpy(iolink_master_port_state(port)->pd_in, resp.pd, resp.pd_len);
        iolink_master_port_state(port)->pd_in_len = resp.pd_len;
        iolink_master_port_state(port)->pd_valid = true;
    }

    iolink_master_route_od(port, resp.od, resp.od_len);

    return IOLINK_MASTER_STATUS_OK;
}

iolink_master_state_t iolink_master_get_state(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state;

    if (port == NULL) {
        return IOLINK_MASTER_STATE_ERROR;
    }

    state = iolink_master_port_const_state(port);
    return state->state;
}

int iolink_master_get_pd_in(const iolink_master_port_t* port, uint8_t* buffer, uint8_t buffer_len,
                            uint8_t* out_len)
{
    const iolink_master_port_state_t* state;

    if ((port == NULL) || (buffer == NULL) || (out_len == NULL)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);

    if (buffer_len < state->pd_in_len) {
        *out_len = state->pd_in_len;
        return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
    }

    *out_len = state->pd_in_len;

    if (!state->pd_valid) {
        return IOLINK_MASTER_STATUS_PENDING;
    }

    (void) memcpy(buffer, state->pd_in, state->pd_in_len);
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_get_od_status(const iolink_master_port_t* port, uint8_t* status)
{
    const iolink_master_port_state_t* state;

    if ((port == NULL) || (status == NULL)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    *status = state->diagnostics.od_status;
    return IOLINK_MASTER_STATUS_OK;
}

uint8_t iolink_master_get_device_status(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state;

    if (port == NULL) {
        return IOLINK_DEVICE_STATUS_FAILURE;
    }

    state = iolink_master_port_const_state(port);
    return state->diagnostics.device_status;
}

int iolink_master_get_diagnostics(const iolink_master_port_t* port,
                                  iolink_master_diagnostics_t* diagnostics)
{
    const iolink_master_port_state_t* state;
    uint32_t error_count;
    uint32_t total_count;

    if ((port == NULL) || (diagnostics == NULL)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    *diagnostics = state->diagnostics;
    diagnostics->supply_voltage_mv = ((state->phy != NULL) && (state->phy->get_voltage_mv != NULL))
                                         ? state->phy->get_voltage_mv(state->phy->user)
                                         : 0;
    diagnostics->short_circuit = ((state->phy != NULL) && (state->phy->is_short_circuit != NULL))
                                     ? state->phy->is_short_circuit(state->phy->user)
                                     : false;
    error_count =
        diagnostics->checksum_errors + diagnostics->send_errors + diagnostics->response_timeouts;
    total_count = state->cycle_count + error_count;
    diagnostics->link_quality_percent =
        (total_count == 0U) ? 100U : (uint8_t) ((state->cycle_count * 100U) / total_count);
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_get_timing(const iolink_master_port_t* port, iolink_master_timing_t* timing)
{
    const iolink_master_port_state_t* state;

    if ((port == NULL) || (timing == NULL)) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    timing->cycle_timer_valid = state->cycle_timer_valid;
    timing->awaiting_response = state->awaiting_response;
    timing->min_cycle_time_100us = state->config.min_cycle_time;
    timing->last_cycle_start_100us = state->last_cycle_start_100us;
    timing->response_deadline_100us = state->response_deadline_100us;

    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_set_pd_out(iolink_master_port_t* port, const uint8_t* data, uint8_t len)
{
    if ((port == NULL) || ((data == NULL) && (len > 0U))) {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if ((len > IOLINK_PD_OUT_MAX_SIZE) ||
        (len != iolink_master_port_state(port)->config.pd_out_len)) {
        return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
    }

    if (len > 0U) {
        (void) memcpy(iolink_master_port_state(port)->pd_out, data, len);
    }
    iolink_master_port_state(port)->pd_out_len = len;
    return IOLINK_MASTER_STATUS_OK;
}
