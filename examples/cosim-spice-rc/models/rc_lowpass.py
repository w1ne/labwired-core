#!/usr/bin/env python3
"""LabWired external_process model: ngspice over rc.cir (see ../rc.cir)."""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "..", "tools", "cosim"))

from labwired_ngspice import serve  # noqa: E402

with open(os.path.join(HERE, "..", "rc.cir")) as f:
    serve(
        f.read(),
        sources={"gpio": "Vgpio"},   # LabWired input  -> SPICE voltage source
        probes={"v_out": "v(out)"},  # LabWired output <- node voltage
        vdd=3.3,
    )
