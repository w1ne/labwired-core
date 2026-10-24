// SPDX-License-Identifier: MIT
#pragma once
#include <cstdint>
namespace codal {
using PVoidCallback = void (*)(void*);
class ScreenIO {
public:
    virtual ~ScreenIO() = default;
    virtual void send(const uint8_t* bytes, unsigned size) = 0;
    virtual void startSend(const uint8_t* bytes, unsigned size, PVoidCallback callback, void* context) = 0;
};
}
