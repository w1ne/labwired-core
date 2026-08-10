#include "obd2.h"

#include <stdbool.h>
#include <stdint.h>

#define REG32(address) (*(volatile uint32_t *)(address))
#define RCC_BASE 0x40021000u
#define RCC_APB2ENR REG32(RCC_BASE + 0x18u)
#define RCC_APB1ENR REG32(RCC_BASE + 0x1Cu)
#define GPIOA_BASE 0x40010800u
#define GPIOA_CRH REG32(GPIOA_BASE + 0x04u)
#define GPIOA_ODR REG32(GPIOA_BASE + 0x0Cu)
#define TIM2_BASE 0x40000000u
#define TIM2_CR1 REG32(TIM2_BASE + 0x00u)
#define TIM2_EGR REG32(TIM2_BASE + 0x14u)
#define TIM2_CNT REG32(TIM2_BASE + 0x24u)
#define TIM2_PSC REG32(TIM2_BASE + 0x28u)
#define TIM2_ARR REG32(TIM2_BASE + 0x2Cu)
#define CAN_BASE 0x40006400u
#define CAN_MCR REG32(CAN_BASE + 0x000u)
#define CAN_MSR REG32(CAN_BASE + 0x004u)
#define CAN_TSR REG32(CAN_BASE + 0x008u)
#define CAN_RF0R REG32(CAN_BASE + 0x00Cu)
#define CAN_BTR REG32(CAN_BASE + 0x01Cu)
#define CAN_TI0R REG32(CAN_BASE + 0x180u)
#define CAN_TDT0R REG32(CAN_BASE + 0x184u)
#define CAN_TDL0R REG32(CAN_BASE + 0x188u)
#define CAN_TDH0R REG32(CAN_BASE + 0x18Cu)
#define CAN_RI0R REG32(CAN_BASE + 0x1B0u)
#define CAN_RDT0R REG32(CAN_BASE + 0x1B4u)
#define CAN_RDL0R REG32(CAN_BASE + 0x1B8u)
#define CAN_RDH0R REG32(CAN_BASE + 0x1BCu)
#define CAN_FMR REG32(CAN_BASE + 0x200u)
#define CAN_FM1R REG32(CAN_BASE + 0x204u)
#define CAN_FS1R REG32(CAN_BASE + 0x20Cu)
#define CAN_FFA1R REG32(CAN_BASE + 0x214u)
#define CAN_FA1R REG32(CAN_BASE + 0x21Cu)
#define CAN_F0R1 REG32(CAN_BASE + 0x240u)
#define CAN_F0R2 REG32(CAN_BASE + 0x244u)

#define LOOP_LIMIT 100000u
#define TSR_RQCP0 (1u << 0)
#define TSR_ABRQ0 (1u << 7)
#define TSR_TME0 (1u << 26)

volatile uint32_t REQUEST_COUNT;
volatile uint32_t RESPONSE_COUNT;
volatile uint32_t DTC_COUNT;
volatile uint32_t LAST_SERVICE;
volatile uint32_t ERROR_COUNT;
volatile uint32_t VIN_TRANSFER_STATE;

static bool wait_msr(uint32_t mask, bool set)
{
    for (uint32_t i = 0; i < LOOP_LIMIT; ++i) {
        if (((CAN_MSR & mask) != 0u) == set) return true;
    }
    ++ERROR_COUNT;
    return false;
}

static void hardware_init(void)
{
    RCC_APB2ENR |= (1u << 0) | (1u << 2); /* AFIO, GPIOA */
    RCC_APB1ENR |= (1u << 25) | (1u << 0); /* CAN1, TIM2 */

    /* PCLK1/TIM2 clock is reset-default HSI 8MHz. PSC=7999 divides it to
     * 1kHz, so the 16-bit CNT is a wrapping millisecond clock. */
    TIM2_CR1 = 0u;
    TIM2_PSC = 7999u;
    TIM2_ARR = 0xFFFFu;
    TIM2_EGR = 1u; /* UG: load prescaler immediately */
    TIM2_CNT = 0u;
    TIM2_CR1 = 1u; /* CEN */

    /* PA11 CAN_RX input pull-up; PA12 CAN_TX 50MHz alternate push-pull. */
    GPIOA_CRH = (GPIOA_CRH & ~((0xFu << 12) | (0xFu << 16))) |
                (0x8u << 12) | (0xBu << 16);
    GPIOA_ODR |= (1u << 11);

    CAN_MCR = 1u;
    if (!wait_msr(1u, true)) return;
    /* HSI reset clock: PCLK1=8MHz. 8MHz/(BRP 1 * (1+BS1 12+BS2 3))=500kbps. */
    CAN_BTR = (11u << 16) | (2u << 20);

    /* One valid 32-bit mask filter: accept standard data frames in hardware;
     * exact 0x7DF/0x7E0 selection is done before any payload access. */
    CAN_FMR |= 1u;
    CAN_FA1R &= ~1u;
    CAN_FS1R |= 1u;
    CAN_FM1R &= ~1u;
    CAN_F0R1 = 0u;
    CAN_F0R2 = (1u << 2) | (1u << 1); /* IDE and RTR must both be zero */
    CAN_FFA1R &= ~1u;
    CAN_FA1R |= 1u;
    CAN_FMR &= ~1u;
    CAN_MCR = 0u;
    (void)wait_msr(1u, false);
}

static uint32_t pack(const uint8_t *data, uint8_t first)
{
    uint32_t word = 0u;
    for (uint8_t i = 0; i < 4u; ++i) word |= (uint32_t)data[first + i] << (8u * i);
    return word;
}

static bool can_send(const obd2_frame_t *frame)
{
    for (uint32_t i = 0; i < LOOP_LIMIT; ++i) {
        if ((CAN_TSR & TSR_TME0) != 0u) {
            CAN_TSR = TSR_RQCP0; /* discard any stale W1C completion */
            CAN_TDL0R = pack(frame->data, 0u);
            CAN_TDH0R = pack(frame->data, 4u);
            CAN_TDT0R = frame->dlc & 0xFu;
            CAN_TI0R = ((frame->id & 0x7FFu) << 21) | 1u;
            for (uint32_t wait = 0; wait < LOOP_LIMIT; ++wait) {
                uint32_t tsr = CAN_TSR;
                if ((tsr & TSR_RQCP0) != 0u) {
                    int status = obd2_tx_status(tsr);
                    CAN_TSR = TSR_RQCP0; /* clear TXOK/ALST/TERR with RQCP W1C */
                    if (status == OBD2_TX_OK) {
                        ++RESPONSE_COUNT;
                        return true;
                    }
                    ++ERROR_COUNT;
                    return false;
                }
            }

            ++ERROR_COUNT;
            CAN_TSR = TSR_ABRQ0;
            for (uint32_t abort_wait = 0; abort_wait < LOOP_LIMIT; ++abort_wait) {
                if ((CAN_TSR & TSR_RQCP0) != 0u) {
                    CAN_TSR = TSR_RQCP0;
                    return false;
                }
            }
            /* Bounded recovery: W1C any completion racing the final poll. */
            CAN_TSR = TSR_RQCP0;
            return false;
        }
    }
    ++ERROR_COUNT;
    return false;
}

static bool can_receive(obd2_frame_t *frame)
{
    if ((CAN_RF0R & 3u) == 0u) return false;
    uint32_t rir = CAN_RI0R;
    uint32_t rdtr = CAN_RDT0R;
    uint32_t low = CAN_RDL0R;
    uint32_t high = CAN_RDH0R;
    CAN_RF0R = (1u << 5); /* Always drain/release, including malformed frames. */
    frame->id = (rir >> 21) & 0x7FFu;
    frame->dlc = (uint8_t)(rdtr & 0xFu);
    for (uint8_t i = 0; i < 4u; ++i) frame->data[i] = (uint8_t)(low >> (8u * i));
    for (uint8_t i = 0; i < 4u; ++i) frame->data[i + 4u] = (uint8_t)(high >> (8u * i));
    if ((rir & 7u) != 0u || frame->dlc > 8u) {
        ++ERROR_COUNT;
        return false;
    }
    return true;
}

int main(void)
{
    obd2_ecu_t ecu;
    obd2_frame_t request, response;
    obd2_init(&ecu);
    DTC_COUNT = ecu.dtc_count;
    hardware_init();

    for (;;) {
        if (can_receive(&request)) {
            ++REQUEST_COUNT;
            if (request.id == 0x7DFu && request.dlc >= 2u) LAST_SERVICE = request.data[1];
            int result = obd2_process(&ecu, &request, TIM2_CNT, &response);
            if (result == OBD2_FRAME_READY) (void)can_send(&response);
            else if (result == OBD2_MALFORMED) ++ERROR_COUNT;
        }
        if (obd2_poll(&ecu, TIM2_CNT, &response) == OBD2_FRAME_READY)
            (void)can_send(&response);
        DTC_COUNT = ecu.dtc_count;
        VIN_TRANSFER_STATE = ecu.vin_transfer_state;
    }
}
