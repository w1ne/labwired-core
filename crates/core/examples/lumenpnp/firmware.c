/* LumenPnP Rev05 diagnostic mission firmware.
 * Executes on STM32F407VE. The pin map comes from the unmodified Opulo
 * motherboard at commit 64cf68d5b7980f095ef11913065e7f45a1e3e447.
 * Custom LabWired diagnostic firmware drives the product's Rev05 pinout.
 * Position comes from emitted STEP pulses. Pick decisions use I2C pressure.
 * The plant returns pressure through physical-contact-dependent I2C registers.
 */
#include <stdint.h>

#define REG(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define RCC 0x40023800u
#define GPIOA 0x40020000u
#define GPIOB 0x40020400u
#define GPIOC 0x40020800u
#define GPIOD 0x40020c00u
#define GPIOE 0x40021000u
#define USART1 0x40011000u
#define I2C1 0x40005400u
#define SYSTICK 0xe000e010u
#define ISR_HZ 10000u
#define TICKS_PER_MS 10u
#define AXES 5u
#define XY_STEPS_MM 40
#define Z_STEPS_MM 40
#define ROTATION_STEPS_REV 1600
#include "assembly-job.generated.h"
#define PICK_THRESHOLD_PA (-25000)
#define RELEASE_THRESHOLD_PA (-250)

enum Axis { X_AXIS, Y_AXIS, Z_AXIS, A_AXIS, B_AXIS };
enum Phase {
    BOOT, HOMING, FEEDER_TRAVEL, PICK_LOWER, EVACUATE, PICK_CONFIRMED,
    LIFT, BOARD_TRAVEL, PLACE_LOWER, VENT, RETRY, COMPLETE, STOPPED
};

typedef struct { uint32_t port; uint8_t bit; } Pin;
static const Pin step_pins[AXES] = {
    {GPIOB, 15}, {GPIOE, 15}, {GPIOE, 7}, {GPIOC, 4}, {GPIOE, 11}
};
static const Pin dir_pins[AXES] = {
    {GPIOB, 14}, {GPIOE, 14}, {GPIOB, 1}, {GPIOA, 4}, {GPIOE, 10}
};
static const Pin enable_pins[6] = {
    {GPIOD, 9}, {GPIOB, 13}, {GPIOE, 9},
    {GPIOB, 0}, {GPIOE, 13}, {GPIOA, 3}
};
static const Pin pump_pins[2] = {{GPIOB, 8}, {GPIOE, 5}};
static const Pin valve_pins[2] = {{GPIOB, 9}, {GPIOE, 6}};
static const Pin limit_pins[3] = {{GPIOC, 6}, {GPIOD, 15}, {GPIOD, 14}};
static const Pin y2_step = {GPIOD, 6};
static const Pin y2_dir = {GPIOD, 7};

/* Fixed, read-only-to-the-runner observation window. It is outside linked
 * data and below the stack. Main and the timer interrupt are the only writers.
 * Fields 6..10 are logical calibrated commanded-step coordinates.
 */
#define OBS ((volatile int32_t *)(uintptr_t)0x20010000u)
static volatile uint32_t milliseconds;
static volatile int32_t position[AXES];
static volatile int32_t pressure_pa[2] = {2147483647, 2147483647};
static volatile unsigned phase, job_index, active_nozzle, attempt;
static volatile unsigned placed, retry_count, fault_code;
static unsigned carrying_part;
static volatile unsigned move_active, home_active, home_axis;
static volatile int32_t home_direction;
static volatile unsigned home_steps_ms;
static volatile uint32_t move_duration, move_elapsed;
static int32_t move_start[AXES], move_delta[AXES], move_target[AXES];
static uint32_t step_accumulator[AXES], interval_steps[AXES];
static int32_t interval_direction[AXES];
static uint32_t timer_subtick, event_count;

static void pin_write(Pin pin, unsigned level) {
    REG(pin.port + 0x18) = 1u << (pin.bit + (level ? 0u : 16u));
}

static unsigned pin_read(Pin pin) {
    return (REG(pin.port + 0x10) >> pin.bit) & 1u;
}

static void pin_mode(Pin pin, unsigned mode) {
    REG(pin.port) = (REG(pin.port) & ~(3u << (pin.bit * 2)))
                 | (mode << (pin.bit * 2));
}

static void pin_af(Pin pin, unsigned af) {
    const uint32_t address = pin.port + 0x20 + (pin.bit / 8) * 4;
    const unsigned shift = (pin.bit % 8) * 4;
    REG(address) = (REG(address) & ~(15u << shift)) | (af << shift);
}

static unsigned wait_bits(uint32_t address, uint32_t mask, uint32_t expected) {
    for (unsigned n = 0; n < 200000; n++) {
        if ((REG(address) & mask) == expected) return 1;
    }
    return 0;
}

static void uart_put(char c) {
    if (wait_bits(USART1, 1u << 7, 1u << 7)) REG(USART1 + 4) = (uint8_t)c;
}

static void uart_text(const char *s) {
    while (*s) uart_put(*s++);
}

static void uart_unsigned(uint32_t n) {
    char buffer[11];
    unsigned length = 0;
    do {
        buffer[length++] = (char)('0' + n % 10);
        n /= 10;
    } while (n);
    while (length) uart_put(buffer[--length]);
}

static void uart_signed(int32_t n) {
    if (n < 0) { uart_put('-'); n = -n; }
    uart_unsigned((uint32_t)n);
}

static void snapshot(void) {
    OBS[0] = 0x504e5035;
    OBS[1] = (int32_t)milliseconds;
    OBS[2] = (int32_t)phase;
    OBS[3] = (int32_t)job_index;
    OBS[4] = (int32_t)active_nozzle;
    OBS[5] = (int32_t)attempt;
    for (unsigned axis = 0; axis < AXES; axis++) OBS[6 + axis] = position[axis];
    OBS[11] = pressure_pa[0];
    OBS[12] = pressure_pa[1];
    OBS[13] = (int32_t)event_count;
    OBS[14] = (int32_t)placed;
    OBS[15] = (int32_t)fault_code;
    OBS[16] = (int32_t)retry_count;
    OBS[17] = (int32_t)move_active;
    OBS[18] = (int32_t)home_active;
}

static void event(const char *message) {
    event_count++;
    uart_text("EV "); uart_unsigned(milliseconds);
    uart_put(' '); uart_text(message);
    uart_text(" job="); uart_unsigned(job_index);
    uart_text(" head="); uart_unsigned(active_nozzle);
    uart_text(" attempt="); uart_unsigned(attempt);
    uart_text(" p0="); uart_signed(pressure_pa[0]);
    uart_text(" p1="); uart_signed(pressure_pa[1]);
    uart_text("\r\n");
    snapshot();
}

static void delay_ms(uint32_t duration) {
    const uint32_t deadline = milliseconds + duration;
    while ((int32_t)(milliseconds - deadline) < 0) __asm volatile("wfi");
}

static void stop(unsigned code) {
    __asm volatile("cpsid i" ::: "memory");
    move_active = home_active = 0;
    for (unsigned axis = 0; axis < AXES; axis++) {
        interval_steps[axis] = 0;
        step_accumulator[axis] = 0;
    }
    fault_code = code;
    phase = STOPPED;
    for (unsigned n = 0; n < 6; n++) pin_write(enable_pins[n], 1);
    for (unsigned n = 0; n < 2; n++) {
        pin_write(pump_pins[n], 0);
        pin_write(valve_pins[n], 0);
    }
    __asm volatile("cpsie i" ::: "memory");
    event("SAFE_STOP");
    for (;;) __asm volatile("wfi");
}

static void prepare_millisecond(void) {
    for (unsigned axis = 0; axis < AXES; axis++) {
        interval_steps[axis] = 0;
        step_accumulator[axis] = 0;
    }
    if (home_active) {
        if (pin_read(limit_pins[home_axis])) {
            home_active = 0;
            return;
        }
        interval_steps[home_axis] = home_steps_ms;
        interval_direction[home_axis] = home_direction;
        return;
    }
    if (!move_active) return;
    if (move_elapsed == move_duration) {
        move_active = 0;
        return;
    }
    /* Cubic rest-to-rest path. Fixed-point u and s use 16 fractional bits.
     * The timer spreads the next 1 ms of requested steps evenly across 10 ticks.
     */
    const uint64_t u = ((uint64_t)(move_elapsed + 1) << 16) / move_duration;
    const uint32_t progress = (uint32_t)((u * u * (3u * 65536u - 2u * u)) >> 32);
    for (unsigned axis = 0; axis < AXES; axis++) {
        const int32_t endpoint = move_start[axis]
            + (int32_t)(((int64_t)move_delta[axis] * progress) / 65536);
        const int32_t delta = endpoint - position[axis];
        interval_direction[axis] = delta < 0 ? -1 : 1;
        interval_steps[axis] = (uint32_t)(delta < 0 ? -delta : delta);
    }
    move_elapsed++;
}

void SysTick_Handler(void) {
    if (timer_subtick == 0) prepare_millisecond();
    unsigned pulse_mask = 0;
    for (unsigned axis = 0; axis < AXES; axis++) {
        step_accumulator[axis] += interval_steps[axis];
        if (step_accumulator[axis] < TICKS_PER_MS) continue;
        step_accumulator[axis] -= TICKS_PER_MS;
        /* Endstop may assert after the homing pulse was queued. */
        if (home_active && axis == home_axis && pin_read(limit_pins[axis])) {
            home_active = 0;
            interval_steps[axis] = 0;
            step_accumulator[axis] = 0;
            continue;
        }
        pin_write(step_pins[axis], 1);
        if (axis == Y_AXIS) pin_write(y2_step, 1);
        position[axis] += interval_direction[axis];
        pulse_mask |= 1u << axis;
    }
    if (pulse_mask) {
        /* Deliberate >100 ns high time, the TMC2226 STEP input minimum.
         * Exact edge spacing is measured by the simulation trace.
         */
        for (volatile unsigned n = 0; n < 30; n++) __asm volatile("nop");
        for (unsigned axis = 0; axis < AXES; axis++) {
            if (!(pulse_mask & (1u << axis))) continue;
            pin_write(step_pins[axis], 0);
            if (axis == Y_AXIS) pin_write(y2_step, 0);
        }
    }
    timer_subtick++;
    if (timer_subtick == TICKS_PER_MS) {
        timer_subtick = 0;
        milliseconds++;
        if ((milliseconds % 10u) == 0) snapshot();
    }
}

static unsigned i2c_wait(uint32_t mask) {
    for (unsigned n = 0; n < 200000; n++) {
        const uint32_t status = REG(I2C1 + 0x14);
        if (status & 0x0f00) { REG(I2C1 + 0x14) = status & ~0x0f00; return 0; }
        if ((status & mask) == mask) return 1;
    }
    return 0;
}

static void i2c_stop(void) { REG(I2C1) |= 1u << 9; }

static unsigned i2c_address(uint8_t address, unsigned read) {
    REG(I2C1) |= 1u << 8;
    if (!i2c_wait(1)) return 0;
    REG(I2C1 + 0x10) = ((uint32_t)address << 1) | read;
    return i2c_wait(2);
}

static unsigned i2c_write(uint8_t address, const uint8_t *bytes, unsigned length) {
    if (!i2c_address(address, 0)) { i2c_stop(); return 0; }
    (void)REG(I2C1 + 0x14); (void)REG(I2C1 + 0x18);
    for (unsigned n = 0; n < length; n++) {
        if (!i2c_wait(1u << 7)) { i2c_stop(); return 0; }
        REG(I2C1 + 0x10) = bytes[n];
    }
    const unsigned ok = i2c_wait(1u << 2);
    i2c_stop();
    return ok;
}

static unsigned i2c_read_byte(uint8_t address, uint8_t reg, uint8_t *value) {
    if (!i2c_write(address, &reg, 1)) return 0;
    REG(I2C1) &= ~((1u << 10) | (1u << 11));
    if (!i2c_address(address, 1)) { i2c_stop(); return 0; }
    __asm volatile("cpsid i" ::: "memory");
    (void)REG(I2C1 + 0x14); (void)REG(I2C1 + 0x18);
    i2c_stop();
    __asm volatile("cpsie i" ::: "memory");
    if (!i2c_wait(1u << 6)) return 0;
    *value = (uint8_t)REG(I2C1 + 0x10);
    return 1;
}

static int32_t read_pressure(unsigned nozzle) {
    const uint8_t mux = (uint8_t)(1u << nozzle);
    const uint8_t convert[2] = {0x30, 0x0a};
    uint8_t status = 8, raw[3];
    if (!i2c_write(0x70, &mux, 1)) stop(20);
    /* PCA9545A channel selection becomes active after the STOP above. */
    if (!i2c_write(0x6d, convert, 2)) stop(21);
    delay_ms(20);
    const uint32_t deadline = milliseconds + 30;
    while (status & 8u) {
        if (!i2c_read_byte(0x6d, 0x30, &status)) stop(22);
        if (!(status & 8u)) break;
        if ((int32_t)(milliseconds - deadline) >= 0) stop(23);
        delay_ms(2);
    }
    for (unsigned n = 0; n < 3; n++) {
        if (!i2c_read_byte(0x6d, (uint8_t)(6 + n), &raw[n])) stop(24);
    }
    int32_t value = ((int32_t)raw[0] << 16) | ((int32_t)raw[1] << 8) | raw[2];
    if (value & 0x800000) value |= (int32_t)0xff000000;
    pressure_pa[nozzle] = value / 32;
    snapshot();
    return pressure_pa[nozzle];
}

static uint32_t magnitude(int32_t value) { return (uint32_t)(value < 0 ? -value : value); }

static void move_to(int32_t x, int32_t y, int32_t z, int32_t a, int32_t b) {
    const int32_t targets[AXES] = {x, y, z, a, b};
    uint32_t duration = 80;
    __asm volatile("cpsid i" ::: "memory");
    for (unsigned axis = 0; axis < AXES; axis++) {
        move_start[axis] = position[axis];
        move_target[axis] = targets[axis];
        move_delta[axis] = targets[axis] - position[axis];
        const uint32_t distance = magnitude(move_delta[axis]);
        uint32_t axis_duration;
        if (axis < Z_AXIS) axis_duration = (distance * 15u + 39u) / 40u;
        else if (axis == Z_AXIS) axis_duration = (distance * 75u + 159u) / 160u;
        else axis_duration = (distance * 15u + 15u) / 16u;
        if (axis_duration > duration) duration = axis_duration;
        pin_write(dir_pins[axis], move_delta[axis] >= 0);
        if (axis == Y_AXIS) pin_write(y2_dir, move_delta[axis] >= 0);
    }
    move_duration = duration;
    move_elapsed = 0;
    move_active = 1;
    __asm volatile("cpsie i" ::: "memory");
    while (move_active) {
        /* Sensor conversions wait in main while SysTick continues motion. */
        if (carrying_part && read_pressure(active_nozzle) >= PICK_THRESHOLD_PA) stop(61);
        __asm volatile("wfi");
    }
    for (unsigned axis = 0; axis < AXES; axis++) {
        if (position[axis] != move_target[axis]) stop(30 + axis);
    }
}

static void home(unsigned axis, int32_t direction, int32_t coordinate) {
    __asm volatile("cpsid i" ::: "memory");
    home_axis = axis;
    home_direction = direction;
    home_steps_ms = 1;
    pin_write(dir_pins[axis], direction > 0);
    if (axis == Y_AXIS) pin_write(y2_dir, direction > 0);
    home_active = 1;
    __asm volatile("cpsie i" ::: "memory");
    const uint32_t deadline = milliseconds + 6000;
    while (home_active) {
        if ((int32_t)(milliseconds - deadline) >= 0) stop(40 + axis);
        __asm volatile("wfi");
    }
    position[axis] = coordinate;
}

static void set_vacuum(unsigned nozzle, unsigned enabled) {
    pin_write(valve_pins[nozzle], enabled);
    pin_write(pump_pins[nozzle], enabled);
}

static int pickup(unsigned nozzle) {
    phase = EVACUATE;
    event("VACUUM_ON");
    set_vacuum(nozzle, 1);
    const uint32_t deadline = milliseconds + 500;
    while ((int32_t)(milliseconds - deadline) < 0) {
        if (read_pressure(nozzle) < PICK_THRESHOLD_PA) {
            phase = PICK_CONFIRMED;
            event("PICK_OK");
            return 1;
        }
    }
    event("PICK_EMPTY");
    set_vacuum(nozzle, 0);
    return 0;
}

static void init(void) {
    REG(RCC) |= 1;
    if (!wait_bits(RCC, 2, 2)) for (;;) {}
    REG(RCC + 8) &= ~3u;
    if (!wait_bits(RCC + 8, 12, 0)) for (;;) {}
    REG(RCC) &= ~(1u << 24);
    if (!wait_bits(RCC, 1u << 25, 0)) for (;;) {}
    REG(RCC + 0x40) |= 1u << 28;
    REG(0x40007000u) |= 1u << 14;
    REG(0x40023c00u) = 5u | (1u << 8) | (1u << 9) | (1u << 10);
    REG(RCC + 8) = (REG(RCC + 8) & ~0x0000fcf3u) | (5u << 10) | (4u << 13);
    REG(RCC + 4) = 16u | (336u << 6) | (7u << 24);
    REG(RCC) |= 1u << 24;
    if (!wait_bits(RCC, 1u << 25, 1u << 25)) for (;;) {}
    REG(RCC + 8) = (REG(RCC + 8) & ~3u) | 2u;
    if (!wait_bits(RCC + 8, 12, 8)) for (;;) {}
    REG(RCC + 0x30) |= 0x1f;
    REG(RCC + 0x40) |= 1u << 21;
    REG(RCC + 0x44) |= 1u << 4;
    (void)REG(RCC + 0x44);
    for (unsigned n = 0; n < 6; n++) { pin_write(enable_pins[n], 1); pin_mode(enable_pins[n], 1); }
    for (unsigned axis = 0; axis < AXES; axis++) {
        pin_write(step_pins[axis], 0); pin_mode(step_pins[axis], 1);
        pin_write(dir_pins[axis], 0); pin_mode(dir_pins[axis], 1);
    }
    pin_write(y2_step, 0); pin_mode(y2_step, 1);
    pin_write(y2_dir, 0); pin_mode(y2_dir, 1);
    for (unsigned n = 0; n < 2; n++) {
        pin_write(pump_pins[n], 0); pin_mode(pump_pins[n], 1);
        pin_write(valve_pins[n], 0); pin_mode(valve_pins[n], 1);
    }
    for (unsigned n = 0; n < 3; n++) pin_mode(limit_pins[n], 0);
    /* UART1 is wired to the RS485 TX signal on this board. Disable the
     * transceiver while capturing controller-side diagnostic bytes. */
    pin_write((Pin){GPIOD, 11}, 0); pin_mode((Pin){GPIOD, 11}, 1);
    pin_write((Pin){GPIOD, 12}, 1); pin_mode((Pin){GPIOD, 12}, 1);
    pin_mode((Pin){GPIOA, 9}, 2); pin_af((Pin){GPIOA, 9}, 7);
    REG(USART1 + 8) = 84;
    REG(USART1 + 12) = (1u << 13) | (1u << 3);
    pin_mode((Pin){GPIOB, 6}, 2); pin_af((Pin){GPIOB, 6}, 4);
    pin_mode((Pin){GPIOB, 7}, 2); pin_af((Pin){GPIOB, 7}, 4);
    REG(GPIOB + 4) |= (1u << 6) | (1u << 7);
    REG(I2C1) = 1u << 15; REG(I2C1) = 0;
    REG(I2C1 + 4) = 42;
    REG(I2C1 + 0x1c) = 210;
    REG(I2C1 + 0x20) = 43;
    REG(I2C1) = 1;
    /* SysTick uses the selected HCLK directly. Its 10 kHz interrupt is the
     * STEP/DIR DDA clock, independent of APB timer multipliers. */
    REG(SYSTICK) = 0;
    REG(SYSTICK + 4) = 168000000u / ISR_HZ - 1;
    REG(SYSTICK + 8) = 0;
    REG(SYSTICK) = 7;
}

int main(void) {
    init();
    snapshot();
    uart_text("LUMENPNP_REV05_DIAGNOSTIC\r\n");
    uart_text("ASSEMBLY_JOB_SHA256="); uart_text(ASSEMBLY_JOB_SHA256); uart_text("\r\n");
    read_pressure(0);
    read_pressure(1);
    if (magnitude(pressure_pa[0]) > 1000 || magnitude(pressure_pa[1]) > 1000) stop(50);
    event("DUAL_SENSOR_AMBIENT_OK");
    for (unsigned n = 0; n < 6; n++) pin_write(enable_pins[n], 0);
    phase = HOMING;
    event("HOME_START");
    home(Z_AXIS, 1, 50 * Z_STEPS_MM);
    home(X_AXIS, -1, 0);
    home(Y_AXIS, -1, 0);
    move_to(0, 0, 25 * Z_STEPS_MM, 0, 0);
    event("HOME_OK");

    for (job_index = 0; job_index < ASSEMBLY_JOB_COUNT; job_index++) {
        const PlacementTask *task = &ASSEMBLY_JOB[job_index];
        active_nozzle = task->head;
        const int32_t x_offset = active_nozzle ? 48 : 0;
        const int32_t contact_z = active_nozzle ? 1944 : 56;
        unsigned success = 0;
        const unsigned max_attempts = task->primary_pocket == task->retry_pocket ? 1 : 2;
        for (attempt = 0; attempt < max_attempts; attempt++) {
            const int32_t slot = (int32_t)(attempt ? task->retry_pocket : task->primary_pocket);
            const int32_t feeder_x = (50 + 8 * slot - x_offset) * XY_STEPS_MM;
            phase = FEEDER_TRAVEL;
            event("MOVE_FEEDER");
            move_to(feeder_x, 60 * XY_STEPS_MM, 25 * Z_STEPS_MM,
                    active_nozzle ? position[A_AXIS] : 0,
                    active_nozzle ? 0 : position[B_AXIS]);
            phase = PICK_LOWER;
            event("LOWER_PICK");
            move_to(position[X_AXIS], position[Y_AXIS], contact_z, position[A_AXIS], position[B_AXIS]);
            success = (unsigned)pickup(active_nozzle);
            carrying_part = success;
            phase = LIFT;
            move_to(position[X_AXIS], position[Y_AXIS], 25 * Z_STEPS_MM, position[A_AXIS], position[B_AXIS]);
            if (success) break;
            retry_count++;
            phase = RETRY;
            event("RETRY_NEXT_POCKET");
        }
        if (!success) stop(60);
        phase = BOARD_TRAVEL;
        event("MOVE_BOARD");
        move_to((task->x_mm - x_offset) * XY_STEPS_MM,
                task->y_mm * XY_STEPS_MM, 25 * Z_STEPS_MM,
                active_nozzle ? position[A_AXIS] : task->rotation_steps,
                active_nozzle ? task->rotation_steps : position[B_AXIS]);
        if (read_pressure(active_nozzle) >= PICK_THRESHOLD_PA) stop(61);
        phase = PLACE_LOWER;
        event("LOWER_PLACE");
        move_to(position[X_AXIS], position[Y_AXIS], contact_z, position[A_AXIS], position[B_AXIS]);
        phase = VENT;
        carrying_part = 0;
        set_vacuum(active_nozzle, 0);
        event("RELEASE");
        const uint32_t release_deadline = milliseconds + 400;
        while (read_pressure(active_nozzle) < RELEASE_THRESHOLD_PA) {
            if ((int32_t)(milliseconds - release_deadline) >= 0) stop(62);
        }
        placed++;
        event("PLACEMENT_CYCLE_OK");
        phase = LIFT;
        move_to(position[X_AXIS], position[Y_AXIS], 25 * Z_STEPS_MM, position[A_AXIS], position[B_AXIS]);
    }
    phase = COMPLETE;
    event("MISSION_COMPLETE");
    for (;;) __asm volatile("wfi");
}
