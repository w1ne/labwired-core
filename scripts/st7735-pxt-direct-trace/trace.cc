// SPDX-License-Identifier: MIT
// Authored host boundary for unchanged Microsoft PXT method fragments.
// Their Microsoft MIT notice is retained separately by run.py and README.md.
#include <cstdint>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

static void require(bool condition, const char* message) {
    if (!condition) throw std::runtime_error(message);
}

struct Pin {
    int level = 1;
    void setDigitalValue(int value) { level = value; }
};

struct SPI {
    Pin& cs;
    Pin& dc;
    bool corrupt;
    std::vector<uint8_t> wire;
    std::vector<int> transfers;
    SPI(Pin& cs_, Pin& dc_, bool corrupt_) : cs(cs_), dc(dc_), corrupt(corrupt_) {}
    void write(int value) {
        require(cs.level == 0 && dc.level == 0, "command pin levels invalid");
        require(wire.empty() && value == 0x2c, "unexpected command");
        wire.push_back(static_cast<uint8_t>(value));
    }
    void transfer(uint8_t* data, int size, uint8_t* rx, int rx_size) {
        require(cs.level == 0 && dc.level == 1, "data pin levels invalid");
        require(size > 0 && size <= 720 && rx == nullptr && rx_size == 0, "invalid transfer extent");
        transfers.push_back(size);
        for (int i = 0; i < size; ++i) {
            auto value = data[i];
            if (corrupt && wire.size() == 1) value ^= 1; // Host-capture mutant only.
            wire.push_back(value);
        }
    }
};

static constexpr int CFG_BOOTLOADER_BOARD_ID = 208;
static uint32_t candidate_board_id;
static uint32_t getConfig(int key, int fallback) {
    require(key == CFG_BOOTLOADER_BOARD_ID && fallback == 0, "unexpected config lookup");
    return candidate_board_id;
}

class WDisplay {
public:
    SPI* spi_;
    Pin* dcPin_;
    Pin* csPin_;
    bool rgb444_ = false;
    uint8_t pal4R_[16] = {0,15,0,0,15,15,0,15,1};
    uint8_t pal4G_[16] = {0,0,15,0,15,0,15,15,3};
    uint8_t pal4B_[16] = {0,0,0,15,0,15,15,15,5};
    WDisplay(SPI* spi, Pin* dc, Pin* cs) : spi_(spi), dcPin_(dc), csPin_(cs) {}
    bool selected() {
#include "predicate.inc"
        return rgb444_;
    }
#include "method.inc"
};

static void numbers(const std::vector<uint8_t>& values) {
    std::cout << "[";
    for (unsigned i = 0; i < values.size(); ++i) {
        if (i) std::cout << ",";
        std::cout << unsigned(values[i]);
    }
    std::cout << "]";
}

static void capture(const char* name, int width, int height,
                    const std::vector<uint8_t>& input,
                    const std::vector<uint8_t>& expected,
                    const std::vector<int>& expected_transfers, bool corrupt) {
    Pin cs, dc;
    SPI spi(cs, dc, corrupt);
    WDisplay display(&spi, &dc, &cs);
    require(input.size() == unsigned(((width + 1) >> 1) * height), "source extent mismatch");
    display.sendIndexedImage444(input.data(), width, height);
    require(cs.level == 1, "CS left selected");
    require(!spi.wire.empty() && spi.wire[0] == 0x2c, "missing RAMWR");
    const std::vector<uint8_t> payload(spi.wire.begin() + 1, spi.wire.end());
    require(payload == expected, "RAMWR mismatch");
    require(spi.transfers == expected_transfers, "transfer batching mismatch");
    std::cout << "{\"name\":\"" << name << "\",\"width\":" << width
              << ",\"height\":" << height
              << ",\"declaredImagePixels\":" << width * height
              << ",\"emittedPixelSlots\":" << payload.size() / 3 * 2
              << ",\"source\":";
    numbers(input);
    std::cout << ",\"ramwr\":";
    numbers(payload);
    std::cout << ",\"transfers\":[";
    for (unsigned i = 0; i < spi.transfers.size(); ++i) {
        if (i) std::cout << ",";
        std::cout << spi.transfers[i];
    }
    std::cout << "],\"csReleased\":true}";
}

int main(int argc, char** argv) {
    try {
        require(argc == 1 || (argc == 2 && std::string(argv[1]) == "--corrupt-data-capture"), "invalid arguments");
        const bool corrupt = argc == 2;
        Pin cs, dc;
        SPI spi(cs, dc, false);
        WDisplay display(&spi, &dc, &cs);
        std::cout << "{\"boardIdPredicate\":[";
        // Original predicate only, not original constructor/branch dispatch.
        const std::vector<uint32_t> yes = {0x239a0000,0x239a0033,0x239affff,0x18591ab9,0x75fdeb5f,0x3f05ba69,0x2dd7a88c,0x2b9e3d05,0x7a236324};
        const std::vector<uint32_t> no = {0,0x23990033,0x239b0033,0x12340033};
        bool first = true;
        for (bool wanted : {true, false}) {
            for (const auto id : wanted ? yes : no) {
                candidate_board_id = id;
                require(display.selected() == wanted, "board-ID predicate mismatch");
                if (!first) std::cout << ",";
                first = false;
                std::cout << "{\"id\":" << id << ",\"selected\":" << (wanted ? "true" : "false") << "}";
            }
        }
        candidate_board_id = 0x239a0033;
        display.spi_ = nullptr;
        require(!display.selected(), "missing SPI selected direct renderer");
        std::cout << "],\"missingSpiRejected\":true,\"cases\":[";
        capture("small-even", 4, 2, {0x21,0x43,0x65,0x87},
                {0xf0,0x00,0xf0,0x00,0xff,0xf0,0xf0,0xf0,0xff,0xff,0xf1,0x35}, {12}, corrupt);
        std::cout << ",";
        // Preserve original odd-width padding: eight emitted pixels for six
        // declared image pixels. This is an observation, not a packing fix.
        capture("odd-width-padding", 3, 2, {0x21,0xa3,0x65,0xb7},
                {0xf0,0x00,0xf0,0x00,0xf0,0x00,0xf0,0xf0,0xff,0xff,0xf0,0x00}, {12}, corrupt);
        std::vector<uint8_t> repeated;
        for (unsigned i = 0; i < 400; ++i) {
            repeated.insert(repeated.end(), {0xf0,0x00,0xf0});
        }
        std::cout << ",";
        // Original lookahead uses 3*(W+1)/2, so W160 flushes after TWO
        // rows despite the three-row buffer/comment. Do not normalize it.
        capture("full-width-batch", 160, 5, std::vector<uint8_t>(400,0x21),
                repeated, {480,480,240}, corrupt);
        repeated.resize(960);
        std::cout << ",";
        capture("odd-width-batch", 159, 4, std::vector<uint8_t>(320,0x21),
                repeated, {720,240}, corrupt);
        std::cout << "]}\n";
        return 0;
    } catch (const std::exception& error) {
        std::cerr << error.what() << "\n";
        return 1;
    }
}
