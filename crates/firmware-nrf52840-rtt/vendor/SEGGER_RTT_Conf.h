#ifndef SEGGER_RTT_CONF_H
#define SEGGER_RTT_CONF_H

// Bare-metal Cortex-M4 configuration for the LabWired RTT demo.
// Channel 0 "Terminal", 1 KiB up / 16 B down, no-block skip mode.
#define SEGGER_RTT_MAX_NUM_UP_BUFFERS   (3)
#define SEGGER_RTT_MAX_NUM_DOWN_BUFFERS (3)
#define BUFFER_SIZE_UP                  (1024)
#define BUFFER_SIZE_DOWN                (16)
#define SEGGER_RTT_PRINTF_BUFFER_SIZE   (64)
#define SEGGER_RTT_MODE_DEFAULT         SEGGER_RTT_MODE_NO_BLOCK_SKIP
#define SEGGER_RTT_MEMCPY_USE_BYTELOOP  1

// Single-threaded demo, and the same vendor source must also compile with the
// host compiler (the workspace host-builds every member). Empty locks.
#define SEGGER_RTT_LOCK()
#define SEGGER_RTT_UNLOCK()

#endif
