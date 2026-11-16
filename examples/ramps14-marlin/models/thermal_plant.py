"""Lumped thermal plant for the RAMPS 1.4 hotend and heated bed.

Each heater is one thermal mass C (J/K) with a loss resistance R (K/W) to
ambient, driven by its MOSFET gate: dT/dt = (P * gate - (T - T_amb) / R) / C.
The gate is the MCU pin level sampled at every co-simulation step, so the
firmware's soft PWM duty becomes average heater power. The thermistor is the
RAMPS T0/T1 circuit: a 100 k EPCOS NTC (B57540G0104, B25/100 = 4092 K) to
GND with a 4.7 k pull-up to 5 V; the output is the ADC pin voltage.

`hotend_probe_detached` (a ui. signal) models a thermistor that has fallen
out of the heater block: the heater still heats the block but the sensor
reads ambient, which is the case thermal runaway protection exists for.

JSONL protocol of the external_process adapter: one request line
{"time_ns", "dt_ns", "inputs"} in, one {"outputs": {...}} line out.
"""
import json
import math
import sys

T_AMB = 25.0
HOTEND = {'p_w': 40.0, 'c_j_per_k': 6.0, 'r_k_per_w': 6.0}
BED = {'p_w': 150.0, 'c_j_per_k': 150.0, 'r_k_per_w': 0.6}
R25, BETA, R_PULL, VCC = 100_000.0, 4092.0, 4_700.0, 5.0


def ntc_volts(t_c):
    r = R25 * math.exp(BETA * (1.0 / (t_c + 273.15) - 1.0 / 298.15))
    return VCC * r / (r + R_PULL)


def advance(t, gate, plant, dt):
    # Exact solution of the linear ODE over dt with the gate held.
    tau = plant['r_k_per_w'] * plant['c_j_per_k']
    t_inf = T_AMB + (plant['p_w'] * plant['r_k_per_w'] if gate else 0.0)
    return t_inf + (t - t_inf) * math.exp(-dt / tau)


def level(value):
    return bool(value) if isinstance(value, bool) else float(value) > 0.5


def serve():
    hotend, bed = T_AMB, T_AMB
    for line in sys.stdin:
        msg = json.loads(line)
        dt = msg.get('dt_ns', 0) / 1e9
        inputs = msg.get('inputs', {})
        hotend = advance(hotend, level(inputs.get('hotend_gate', False)), HOTEND, dt)
        bed = advance(bed, level(inputs.get('bed_gate', False)), BED, dt)
        sensed = T_AMB if level(inputs.get('hotend_probe_detached', 0.0)) else hotend
        out = {'hotend_v': ntc_volts(sensed), 'bed_v': ntc_volts(bed)}
        sys.stdout.write(json.dumps({'outputs': out}) + '\n')
        sys.stdout.flush()


if __name__ == '__main__':
    serve()
