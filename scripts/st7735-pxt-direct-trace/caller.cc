// SPDX-License-Identifier: MIT
// Authored boundaries around exact Microsoft MIT fragments (notice in README).
#include <cstdint>
#include <cstring>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

static void require(bool ok, const char* message) {
    if (!ok) throw std::runtime_error(message);
}
static void oops(int) { throw std::runtime_error("unsupported image bpp"); }
static constexpr int PANIC_SCREEN_ERROR = 42;
static void target_panic(int) { throw std::runtime_error("dimension/bpp panic"); }

#include "image-header.inc"
struct BoxedBuffer { uint8_t* data; unsigned length; };
class Image {
public:
    std::vector<uint8_t> storage;
    BoxedBuffer boxed;
    BoxedBuffer* buffer = &boxed;
    Image(int w, int h, int bpp = 4)
        : storage(sizeof(ImageHeader) + w * (((h * bpp + 31) >> 5) << 2)),
          boxed{storage.data(), unsigned(storage.size())} {
        header()->magic = 0x87;
        header()->bpp = bpp;
        header()->width = w;
        header()->height = h;
        // Explicit column/row sentinels and nonzero padding, no repacking.
        std::memset(pix(), 0xff, pixLength());
        if (bpp == 4) {
            for (int x = 0; x < w; ++x) for (int y = 0; y < h; ++y) {
                uint8_t* p = pix(x, y);
                const unsigned value = 1 + (x * 3 + y * 5) % 8;
                if (y & 1) *p = uint8_t((*p & 0x0f) | (value << 4));
                else *p = uint8_t((*p & 0xf0) | value);
            }
        }
    }
#include "image-accessors.inc"
};
using Image_ = Image*;

struct Pin {
    int level = 1;
    void setDigitalValue(int value) { level = value; }
};
static bool corrupt_capture;
struct SPI {
    Pin& cs;
    Pin& dc;
    std::vector<std::vector<uint8_t>> payloads;
    std::vector<std::vector<int>> transfers;
    void write(int value) {
        require(cs.level == 0 && dc.level == 0 && value == 0x2c, "caller command mismatch");
        payloads.emplace_back();
        transfers.emplace_back();
    }
    void transfer(uint8_t* src, int n, uint8_t* rx, int rx_size) {
        require(cs.level == 0 && dc.level == 1 && n > 0 && n <= 720 && !rx && !rx_size,
                "caller data boundary mismatch");
        require(!payloads.empty(), "caller missing RAMWR");
        auto& bytes = payloads.back();
        transfers.back().push_back(n);
        for (int i = 0; i < n; ++i) {
            uint8_t value = src[i];
            if (corrupt_capture && payloads.size() == 1 && bytes.empty()) value ^= 1;
            bytes.push_back(value);
        }
    }
};

class WDisplay {
public:
    int width, height, displayHeight;
    bool doubleSize = false, inUpdate = false, rgb444_ = true, smart = false, newPalette = true;
    Image_ lastStatus = nullptr;
    void* lcd = this;
    uint32_t palette[16] = {};
    uint32_t* currPalette = palette;
    std::vector<uint8_t> storage;
    uint8_t* screenBuf;
    Pin cs, dc;
    SPI spi{cs, dc, {}, {}};
    SPI* spi_ = &spi;
    Pin* csPin_ = &cs;
    Pin* dcPin_ = &dc;
    uint8_t pal4R_[16] = {0,15,0,0,15,15,0,15,1,0,0,0,0,0,0,2};
    uint8_t pal4G_[16] = {0,0,15,0,15,0,15,15,3,0,0,0,0,0,0,4};
    uint8_t pal4B_[16] = {0,0,0,15,0,15,15,15,5,0,0,0,0,0,0,6};
    int fallbackCalls = 0, waits = 0, mainWindows = 0, statusWindows = 0;
    std::vector<size_t> copies;
    WDisplay(int w, int h, int main_h, bool doubled = false)
        : width(w), height(h), displayHeight(main_h),
          doubleSize(doubled),
          storage((doubled ? (w >> 1) * (h >> 1) : w * h) / 2 + 20),
          screenBuf(storage.data()) {}
    void waitForSendDone() { ++waits; }
    void setAddrMain() { ++mainWindows; }
    void setAddrStatus() { ++statusWindows; }
    int sendIndexedImage(const uint8_t*, unsigned, unsigned, uint32_t*) {
        ++fallbackCalls; return 0; // Explicit authored fallback, not CODAL.
    }
#include "method.inc"
};
static WDisplay* selected;
static WDisplay* getWDisplay() { return selected; }
static void* checked_copy(void* dst, const void* src, size_t n) {
    require(selected && dst == selected->screenBuf, "unexpected copy destination");
    selected->copies.push_back(n);
    require(n <= selected->storage.size(), "screenBuf copy extent rejected");
    return std::memcpy(dst, src, n);
}
// Exact fragment bytes retained. This authored interception rejects an
// oversized copy before mutation; it is not an original runtime memory guard.
#define memcpy checked_copy
#include "caller.inc"
#undef memcpy

static bool compiled() {
#ifdef USE_RGB444
    return true;
#else
    return false;
#endif
}
// Independent literal RGB444 palette triples for indices0..15.
static constexpr uint16_t colours[16] = {
    0x000,0xf00,0x0f0,0x00f,0xff0,0xf0f,0x0ff,0xfff,
    0x135,0,0,0,0,0,0,0x246
};
static std::vector<uint8_t> expected_bytes(Image& img, int emitted_pixels) {
    std::vector<uint8_t> expected;
    require(emitted_pixels % 2 == 0 && emitted_pixels / 2 <= img.pixLength(),
            "oracle input extent mismatch");
    for (int i = 0; i < emitted_pixels / 2; ++i) {
        const auto source = img.pix()[i];
        const int stride = ((img.height() + 7) / 8) * 4;
        const int column = i / stride, y = (i % stride) * 2;
        const unsigned low = y < img.height() ? 1 + (column * 3 + y * 5) % 8 : 15;
        const unsigned high = y + 1 < img.height() ? 1 + (column * 3 + (y + 1) * 5) % 8 : 15;
        require(source == (low | (high << 4)), "original Image accessor storage mismatch");
        const auto first = colours[source & 15], second = colours[source >> 4];
        expected.push_back(uint8_t(first >> 4));
        expected.push_back(uint8_t((first << 4) | (second >> 8)));
        expected.push_back(uint8_t(second));
    }
    return expected;
}
static bool padding_selected(Image& img, int emitted_pixels) {
    for (int i = 0; i < emitted_pixels; ++i) {
        const auto byte = img.pix()[i / 2];
        if (((i & 1) ? byte >> 4 : byte & 15) == 15) return true;
    }
    return false;
}
static void image_metadata(Image* img) {
    if (!img) { std::cout << "null"; return; }
    std::cout << "{\"width\":" << img->width() << ",\"height\":" << img->height()
              << ",\"bpp\":" << img->bpp() << ",\"columnStride\":" << img->byteHeight()
              << ",\"pixelStorageBytes\":" << img->pixLength() << "}";
}
static void capture(const char* name, WDisplay& d, Image* main, Image* status,
                    bool expect_direct, const char* error = nullptr) {
    selected = &d;
    d.lastStatus = status;
    std::string failure;
    try { updateScreen(main); } catch (const std::exception& e) { failure = e.what(); }
    require(failure == (error ? error : ""), "unexpected caller acceptance/rejection");
    if (!error) {
        require(!d.inUpdate, "caller left reentry flag set");
        std::vector<size_t> expected_copies;
        if (main) expected_copies.push_back(size_t(main->width() * ((main->height() + 7) / 8) * 4));
        if (status) expected_copies.push_back(size_t(status->width() * ((status->height() + 7) / 8) * 4));
        require(d.copies == expected_copies, "original caller copy extent mismatch");
        if (expect_direct && compiled()) {
            std::vector<Image*> images;
            std::vector<int> heights;
            if (main) { images.push_back(main); heights.push_back(d.displayHeight); }
            if (status) { images.push_back(status); heights.push_back(d.height - d.displayHeight); }
            require(d.spi.payloads.size() == images.size(), "caller direct dispatch mismatch");
            for (unsigned i = 0; i < images.size(); ++i) {
                require(d.spi.payloads[i] == expected_bytes(*images[i], d.width * heights[i]),
                        "caller RAMWR mismatch");
                // For W160 the original lookahead flushes after two rows.
                std::vector<int> transfers(heights[i] / 2, 480);
                if (heights[i] & 1) transfers.push_back(240);
                require(d.spi.transfers[i] == transfers, "caller transfer sizes mismatch");
            }
            for (unsigned i = 0; i < images.size(); ++i)
                require(padding_selected(*images[i], d.width * heights[i]) == images[i]->hasPadding(),
                        "caller padding selection mismatch");
            require(d.cs.level == 1 && d.fallbackCalls == 0, "caller direct completion mismatch");
            if (status) require(d.lastStatus == nullptr && d.statusWindows == 1, "status completion mismatch");
        } else {
            require(d.spi.payloads.empty(), "unexpected direct dispatch");
            require(d.fallbackCalls == int(bool(main)) + int(bool(status)), "fallback dispatch mismatch");
        }
    } else {
        require(d.spi.payloads.empty(), "rejected input emitted SPI");
        require(d.inUpdate, "panic boundary unexpectedly repaired original reentry state");
    }
    const int emitted = main ? d.width * d.displayHeight : 0;
    std::cout << "{\"name\":\"" << name << "\",\"mainImage\":";
    image_metadata(main);
    std::cout << ",\"statusImage\":";
    image_metadata(status);
    std::cout << ",\"displayWidth\":" << d.width << ",\"displayHeight\":" << d.height
              << ",\"mainHeight\":" << d.displayHeight << ",\"copiedBytes\":[";
    for (unsigned i = 0; i < d.copies.size(); ++i) { if (i) std::cout << ","; std::cout << d.copies[i]; }
    std::cout << "],\"allocationBytes\":" << d.storage.size()
              << ",\"directFrames\":" << d.spi.payloads.size()
              << ",\"fallbackCalls\":" << d.fallbackCalls
              << ",\"inUpdateAfter\":" << (d.inUpdate ? "true" : "false")
              << ",\"waits\":" << d.waits
              << ",\"mainWindows\":" << d.mainWindows
              << ",\"statusWindows\":" << d.statusWindows
              << ",\"mainPaddingSelected\":" << (main && expect_direct && compiled() && !error && padding_selected(*main, emitted) ? "true" : "false")
              << ",\"statusPaddingSelected\":" << (status && expect_direct && compiled() && !error && padding_selected(*status, d.width * (d.height - d.displayHeight)) ? "true" : "false")
              << ",\"rejected\":" << (error ? "true" : "false")
              << ",\"rejectionReason\":\"" << failure << "\",\"frames\":[";
    for (unsigned frame = 0; frame < d.spi.payloads.size(); ++frame) {
        if (frame) std::cout << ",";
        std::cout << "{\"ramwr\":[";
        const auto& bytes = d.spi.payloads[frame];
        for (unsigned i = 0; i < bytes.size(); ++i) { if (i) std::cout << ","; std::cout << unsigned(bytes[i]); }
        std::cout << "],\"transfers\":[";
        const auto& transfers = d.spi.transfers[frame];
        for (unsigned i = 0; i < transfers.size(); ++i) { if (i) std::cout << ","; std::cout << transfers[i]; }
        std::cout << "],\"csReleased\":true}";
    }
    std::cout << "]}";
}
int main(int argc, char** argv) {
    try {
        require(argc == 1 || (argc == 2 && std::string(argv[1]) == "--corrupt-data-capture"), "invalid argument");
        corrupt_capture = argc == 2;
        std::cout << "{\"rgb444Compiled\":" << (compiled() ? "true" : "false") << ",\"cases\":[";
        Image full(160,128);
        WDisplay a(160,128,128); capture("full-aligned",a,&full,nullptr,true);
        std::cout << ","; Image partial(160,5);
        WDisplay b(160,128,5); capture("partial-padded",b,&partial,nullptr,true);
        std::cout << ","; Image main120(160,120), bar8(160,8);
        WDisplay c(160,128,120); capture("aligned-status",c,&main120,&bar8,true);
        std::cout << ","; Image main125(160,125), bar3(160,3);
        WDisplay d(160,128,125); capture("padded-status",d,&main125,&bar3,true);
        std::cout << ","; WDisplay e(160,128,128); e.rgb444_=false;
        capture("predicate-off-fallback",e,&full,nullptr,false);
        std::cout << ","; WDisplay f(160,128,128); f.lcd=nullptr;
        capture("missing-lcd-fallback",f,&full,nullptr,false);
        std::cout << ","; Image half(80,64); WDisplay g(160,128,128,true);
        capture("doubled-fallback",g,&half,nullptr,false);
        std::cout << ","; Image wrong(159,128); WDisplay h(160,128,128);
        capture("dimension-negative",h,&wrong,nullptr,true,"dimension/bpp panic");
        std::cout << ","; Image mono(160,128,1); WDisplay j(160,128,128);
        capture("bpp-negative",j,&mono,nullptr,true,"dimension/bpp panic");
        std::cout << ","; Image tallpad(160,1); WDisplay k(160,1,1);
        capture("padded-allocation-negative",k,&tallpad,nullptr,true,"screenBuf copy extent rejected");
        selected=nullptr; updateScreen(&full);
        WDisplay reentry(160,128,128); reentry.inUpdate=true; selected=&reentry;
        updateScreen(&full);
        require(reentry.inUpdate && reentry.copies.empty() && reentry.spi.payloads.empty()
                && reentry.fallbackCalls == 0, "reentry negative mismatch");
        std::cout << ",{\"name\":\"missing-display-and-reentry\",\"inUpdateAfter\":true,"
                  << "\"copiedBytes\":[],\"directFrames\":0,\"fallbackCalls\":0}";
        std::cout << "]}\n";
        return 0;
    } catch (const std::exception& e) { std::cerr << e.what() << "\n"; return 1; }
}
