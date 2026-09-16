#ifndef SEGGER_RTT_CONF_H
#define SEGGER_RTT_CONF_H

// Bare-metal Cortex-M4 configuration shared by the LabWired RTT fixtures.
// Defaults: channel 0 "Terminal", 1 KiB up / 16 B down, no-block skip mode.
// Each value is wrapped in #ifndef so a fixture build can override it with -D
// (the blocking fixture sets BUFFER_SIZE_UP=16 and
// SEGGER_RTT_MODE_DEFAULT=SEGGER_RTT_MODE_BLOCK_IF_FIFO_FULL).
#ifndef SEGGER_RTT_MAX_NUM_UP_BUFFERS
#define SEGGER_RTT_MAX_NUM_UP_BUFFERS   (3)
#endif
#ifndef SEGGER_RTT_MAX_NUM_DOWN_BUFFERS
#define SEGGER_RTT_MAX_NUM_DOWN_BUFFERS (3)
#endif
#ifndef BUFFER_SIZE_UP
#define BUFFER_SIZE_UP                  (1024)
#endif
#ifndef BUFFER_SIZE_DOWN
#define BUFFER_SIZE_DOWN                (16)
#endif
#ifndef SEGGER_RTT_PRINTF_BUFFER_SIZE
#define SEGGER_RTT_PRINTF_BUFFER_SIZE   (64)
#endif
#ifndef SEGGER_RTT_MODE_DEFAULT
#define SEGGER_RTT_MODE_DEFAULT         SEGGER_RTT_MODE_NO_BLOCK_SKIP
#endif
#ifndef SEGGER_RTT_MEMCPY_USE_BYTELOOP
#define SEGGER_RTT_MEMCPY_USE_BYTELOOP  1
#endif

// Single-threaded demo, and the same vendor source must also compile with the
// host compiler (the workspace host-builds every member). Empty locks.
#define SEGGER_RTT_LOCK()
#define SEGGER_RTT_UNLOCK()

#endif
