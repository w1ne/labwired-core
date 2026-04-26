#!/usr/bin/env python3
"""Capture from /dev/ttyACM1 with proper inter-byte timing.

Reads byte-by-byte with select() so USB CDC packet boundaries don't
truncate the stream. Stops after `idle_timeout` seconds of silence.
"""
import os, sys, select, termios, struct, fcntl, time

DEV = sys.argv[1] if len(sys.argv) > 1 else "/dev/ttyACM1"
BAUD = int(sys.argv[2]) if len(sys.argv) > 2 else 115200
OUT = sys.argv[3] if len(sys.argv) > 3 else "/tmp/serial_capture.bin"
IDLE = float(sys.argv[4]) if len(sys.argv) > 4 else 3.0

# Open with O_NONBLOCK so reads don't hang.
fd = os.open(DEV, os.O_RDONLY | os.O_NONBLOCK)

# Configure the tty: raw mode, 115200 8N1, no flow control, no echo.
# termios c_cflag bits.
import termios
attrs = termios.tcgetattr(fd)
iflag, oflag, cflag, lflag, ispeed, ospeed, cc = attrs

# Look up baud constant.
BAUD_CONSTS = {
    9600:    termios.B9600,
    38400:   termios.B38400,
    57600:   termios.B57600,
    115200:  termios.B115200,
    230400:  termios.B230400,
}
if BAUD not in BAUD_CONSTS:
    print(f"unsupported baud {BAUD}", file=sys.stderr)
    sys.exit(1)
b = BAUD_CONSTS[BAUD]
ispeed = b
ospeed = b
# CS8, no parity, no flow control.
cflag = b | termios.CS8 | termios.CREAD | termios.CLOCAL
iflag = 0
oflag = 0
lflag = 0
cc[termios.VMIN] = 0
cc[termios.VTIME] = 0
termios.tcsetattr(fd, termios.TCSANOW, [iflag, oflag, cflag, lflag, ispeed, ospeed, cc])
termios.tcflush(fd, termios.TCIOFLUSH)

print(f"capturing from {DEV} @ {BAUD} -> {OUT} (idle stop = {IDLE}s)", file=sys.stderr)

with open(OUT, "wb") as out:
    last_data = time.time()
    total = 0
    while True:
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            try:
                chunk = os.read(fd, 4096)
            except BlockingIOError:
                chunk = b""
            if chunk:
                out.write(chunk)
                out.flush()
                total += len(chunk)
                last_data = time.time()
        if time.time() - last_data > IDLE and total > 0:
            break
        if time.time() - last_data > 30 and total == 0:
            print("no data after 30s — giving up", file=sys.stderr)
            break

os.close(fd)
print(f"captured {total} bytes", file=sys.stderr)
