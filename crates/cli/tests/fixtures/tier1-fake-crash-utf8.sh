#!/bin/sh
python3 << 'PYEOF'
import sys
for i in range(200):
    sys.stderr.buffer.write(b'\xe2\x9d\x84')
sys.exit(3)
PYEOF
