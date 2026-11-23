# BLE Pong compiled fixture

`esp32c3-ble-pong.ino` is the exact source paired with the committed
`esp32c3-ble-pong-flash.bin`. Keep the source bytes unchanged, including comments,
unless the image is rebuilt with the procedure in
[`fixture_ble_pong_provenance.rs`](../esp/fixture_ble_pong_provenance.rs).

| Artifact | SHA-256 |
| --- | --- |
| Sketch | `3b4d793e9b129784e94002a04dd00dc1ba0a5191a8700e413a47d2301d69245f` |
| Flash image | `1789ebcb62fcd6d72dd4eb88718a8efd2c6da74ff5df0793509e3892653f66e0` |

The source is restored from the compiled version recorded before the comment-only
edit in `9e8989aa4`. The firmware image and both provenance constants are unchanged.

## Sharp sensor model scope

The sensor comments inside that historical sketch predate the corrected
[`gp2y0a21` catalog descriptor](../../../../configs/devices/gp2y0a21.yaml).
The current descriptor follows Sharp's typical white-paper output graph: about
2.3 V at 10 cm (2854 counts on a 12-bit, 3.3 V scale) and 0.4 V at 80 cm
(496 counts). Its 0.4 V far clamp is a model assumption beyond the datasheet's
80 cm measuring range.

The sketch's `RAW_NEAR = 3600` and `RAW_FAR = 650` are application normalization
endpoints. They are not predicted sensor outputs. This explanation is kept beside
the paired artifact so a documentation correction cannot silently detach the
source from its recorded compiled image.
