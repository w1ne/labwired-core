#!/usr/bin/env python3
"""Echo plant over the shared-memory transport: v_out = 3.3 when gpio is high."""
from labwired_shm import serve

serve(lambda t, dt, i: {"v_out": 3.3 if i.get("gpio") else 0.0})
