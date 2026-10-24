// SPDX-License-Identifier: MIT
#pragma once
#include <stdexcept>
inline void fiber_sleep(int) {} // Trace-only: no physical reset/delay claim.
inline void fiber_wait_for_event(int, int) {
    throw std::runtime_error("unexpected unfinished driver transfer");
}
