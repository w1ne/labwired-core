// SPDX-License-Identifier: MIT
// Executes the original pinned source against an explicitly artificial host
// transport. Does not qualify SAM/DMA/IRQ/fibers/module glass or timing.
#include "ST7735.h"
#include <deque>
#include <iostream>
#include <sstream>
#include <utility>
#include <vector>

namespace codal {
EventBus bus;
EventBus* EventModel::defaultEventBus = &bus;
void emit_trace_event(const Event& event) {
    // Copy handlers: nested completion events must not invalidate iteration.
    for (auto handler : bus.handlers) {
        if (handler.source == event.source && handler.value == event.value) {
            handler.call(event); // Copy construction does not re-emit.
        }
    }
}
}

static void require(bool condition, const char* message) {
    if (!condition) throw std::runtime_error(message);
}

class CaptureIO : public codal::ScreenIO {
public:
    struct Byte { bool data; uint8_t value; };
    struct Pending { codal::PVoidCallback callback; void* context; };
    codal::Pin& cs;
    codal::Pin& dc;
    std::vector<Byte> bytes;
    std::deque<Pending> pending;
    bool corrupt_window;
    CaptureIO(codal::Pin& cs_, codal::Pin& dc_, bool corrupt_window_)
        : cs(cs_), dc(dc_), corrupt_window(corrupt_window_) {}
    void send(const uint8_t* data, unsigned size) override {
        require(cs.level == 0, "driver wrote with CS inactive");
        for (unsigned i = 0; i < size; ++i) {
            auto value = data[i];
            if (corrupt_window && bytes.size() == 1 && !bytes.front().data && bytes.front().value == 0x2a) {
                value ^= 1; // Named host-capture mutant, never production input.
            }
            bytes.push_back({dc.level != 0, value});
        }
    }
    void startSend(const uint8_t* data, unsigned size, codal::PVoidCallback callback, void* context) override {
        send(data, size);
        pending.push_back({callback, context});
    }
    void drain() {
        unsigned count = 0;
        while (!pending.empty()) {
            require(++count <= 32, "callback bound exceeded");
            const auto next = pending.front();
            pending.pop_front();
            next.callback(next.context);
        }
    }
    std::vector<std::pair<uint8_t, std::vector<uint8_t>>> commands() const {
        std::vector<std::pair<uint8_t, std::vector<uint8_t>>> out;
        for (const auto byte : bytes) {
            if (!byte.data) out.push_back({byte.value, {}});
            else {
                require(!out.empty(), "data without command");
                out.back().second.push_back(byte.value);
            }
        }
        return out;
    }
};

int main(int argc, char** argv) {
    try {
        require(argc == 1 || (argc == 2 && std::string(argv[1]) == "--corrupt-window-capture"), "invalid trace arguments");
        std::cout << "[";
        bool first = true;
        for (unsigned width : {3u, 4u, 5u}) {
            codal::bus.handlers.clear();
            codal::Pin cs, dc;
            CaptureIO io(cs, dc, argc == 2);
            codal::ST7735 driver(io, cs, dc);
            driver.init();
            driver.configure(0, 0xffffff);
            io.bytes.clear();
            // Original driver's coordinate convention: CASET from y,
            // RASET from x; width grows the latter, not the former.
            driver.setAddrWindow(7, 11, width, 2);
            alignas(4) const uint8_t indexed[] = {0x21, 0x43, 0x65, 0x87, 0xa9, 0};
            uint32_t palette[16] = {};
            palette[1] = 0xff0000;
            palette[2] = 0x00ff00;
            palette[3] = 0x0000ff;
            palette[4] = 0xffff00;
            palette[5] = 0xff00ff;
            palette[6] = 0x00ffff;
            palette[7] = 0xffffff;
            palette[8] = 0x123456;
            require(driver.sendIndexedImage(indexed, width, 2, palette) == DEVICE_OK, "driver refused image");
            io.drain();
            driver.waitForSendDone();
            require(cs.level == 1, "driver left CS selected");
            const auto commands = io.commands();
            require(commands.size() == 4, "unexpected command count");
            require(commands[0].first == 0x2a && commands[0].second == std::vector<uint8_t>({0,11,0,12}), "CASET mismatch");
            require(commands[1].first == 0x2b && commands[1].second == std::vector<uint8_t>({0,7,0,static_cast<uint8_t>(6+width)}), "RASET mismatch");
            std::vector<uint8_t> lut(128, 0);
            for (unsigned i : {1u,4u,5u,7u}) lut[i] = 63;
            for (unsigned i : {2u,4u,6u,7u}) lut[32+i] = 63;
            for (unsigned i : {3u,5u,6u,7u}) lut[96+i] = 63;
            lut[8] = 4; lut[40] = 13; lut[104] = 21;
            require(commands[2].first == 0x2d && commands[2].second == lut, "RGBSET mismatch");
            const std::vector<uint8_t> expected = {
                0x11,0x12,0x22,0x33,0x34,0x44,0x55,0x56,0x66,0x77,0x78,0x88,0x99,0x9a,0xaa
            };
            require(commands[3].first == 0x2c && commands[3].second == std::vector<uint8_t>(expected.begin(), expected.begin()+width*3), "RAMWR packing mismatch");
            if (!first) std::cout << ",";
            first = false;
            std::cout << "{\"width\":" << width << ",\"height\":2,\"commands\":[";
            bool first_command = true;
            for (const auto& command : commands) {
                if (!first_command) std::cout << ",";
                first_command = false;
                std::cout << "{\"opcode\":" << unsigned(command.first) << ",\"data\":[";
                for (unsigned i=0; i<command.second.size(); ++i) {
                    if (i) std::cout << ",";
                    std::cout << unsigned(command.second[i]);
                }
                std::cout << "]}";
            }
            std::cout << "]}";
        }
        std::cout << "]\n";
        return 0;
    } catch (const std::exception& error) {
        std::cerr << error.what() << "\n";
        return 1;
    }
}
