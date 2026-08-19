#!/usr/bin/env python3
"""Host sender for the BRD2709A UART XMODEM bootloader.

Opens the J-Link VCOM, knocks LWBL until the bootloader replies with 'C',
then sends the app-slot .bin as XMODEM-1K.

  python3 bench_xmodem.py --port /dev/cu.usbmodem0004403389371 firmware.bin
"""
from __future__ import annotations

import argparse
import os
import select
import sys
import termios
import time
import tty

SOH, STX, EOT, ACK, NAK, CAN, CRC_NAK = 0x01, 0x02, 0x04, 0x06, 0x15, 0x18, 0x43
KNOCK = b"LWBL"


def crc16_xmodem(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc


def encode_packets(payload: bytes) -> list[bytes]:
    block_size = 1024
    total = max(1, (len(payload) + block_size - 1) // block_size)
    packets = []
    for i in range(total):
        block = (i + 1) & 0xFF
        data = payload[i * block_size : (i + 1) * block_size]
        data = data + b"\x1a" * (block_size - len(data))
        crc = crc16_xmodem(data)
        packets.append(bytes([STX, block, 0xFF - block]) + data + bytes([crc >> 8, crc & 0xFF]))
    return packets


def open_serial(port: str, baud: int) -> int:
    fd = os.open(port, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    attrs = termios.tcgetattr(fd)
    tty.setraw(fd)
    speed = {
        9600: termios.B9600,
        115200: termios.B115200,
        230400: termios.B230400,
    }[baud]
    attrs[0] = 0  # iflag
    attrs[1] = 0  # oflag
    attrs[2] = termios.CS8 | termios.CREAD | termios.CLOCAL
    attrs[3] = 0  # lflag
    attrs[4] = speed
    attrs[5] = speed
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    termios.tcflush(fd, termios.TCIOFLUSH)
    return fd


def read_byte(fd: int, timeout: float) -> int | None:
    ready, _, _ = select.select([fd], [], [], timeout)
    if not ready:
        return None
    data = os.read(fd, 1)
    return data[0] if data else None


def write_all(fd: int, data: bytes) -> None:
    view = memoryview(data)
    while view:
        ready, w, _ = select.select([], [fd], [], 2)
        if not w:
            raise TimeoutError("serial write timeout")
        try:
            n = os.write(fd, view)
        except BlockingIOError:
            continue
        if n == 0:
            raise OSError("serial write returned 0")
        view = view[n:]


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("image")
    p.add_argument("--port", default="/dev/cu.usbmodem0004403389371")
    p.add_argument("--baud", type=int, default=115200)
    args = p.parse_args()
    payload = open(args.image, "rb").read()
    if not payload:
        print("empty image", file=sys.stderr)
        return 2
    fd = open_serial(args.port, args.baud)
    print(f"knocking LWBL on {args.port} — press RESET (or hold BTN0 at reset)", flush=True)
    started = False
    deadline = time.time() + 15
    while time.time() < deadline:
        write_all(fd, KNOCK)
        b = read_byte(fd, 0.1)
        if b == CRC_NAK:
            started = True
            break
    if not started:
        print("no 'C' from bootloader", file=sys.stderr)
        return 1
    packets = encode_packets(payload)
    print(f"sending {len(payload)} bytes in {len(packets)} XMODEM-1K blocks", flush=True)
    for i, pkt in enumerate(packets):
        write_all(fd, pkt)
        ack = read_byte(fd, 10)
        if ack != ACK:
            print(f"block {i + 1} not ACKed (got {ack})", file=sys.stderr)
            return 1
        print(f"  {i + 1}/{len(packets)}", flush=True)
    write_all(fd, bytes([EOT]))
    if read_byte(fd, 10) != ACK:
        print("EOT not ACKed", file=sys.stderr)
        return 1
    print("OK — bootloader should jump to 0x08008000")
    os.close(fd)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
