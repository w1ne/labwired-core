// SPDX-License-Identifier: MIT
// Host trace boundary only; not a hardware implementation.
#pragma once
#include <cstdint>
#include <cstring>
#include <stdexcept>
#define DEVICE_OK 0
#define DEVICE_BUSY -1
#define DEVICE_ID_DISPLAY 1000
inline void target_panic(int) { throw std::runtime_error("CODAL panic"); }
namespace codal {
class CodalComponent {};
class Pin {
public:
    int level = 1;
    void setDigitalValue(int value) { level = value; }
};
}
