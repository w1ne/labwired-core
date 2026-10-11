"""Deterministic reduced-order smart-basket power plant, not firmware or Linux.

Model parameters arrive as flat config.* INPUT signals. ExternalProcessAdapter
only forwards time_ns, dt_ns and inputs; manifest config is not a parameter API.
See examples/smart-basket-power/README.md for assumptions and interval semantics.
"""
from __future__ import annotations

import json
import math
import sys

# All defaults are assumptions, not authoritative application/component values.
DEFAULTS = {
    'capacity_mah': 2000.0, 'initial_soc': 0.6, 'usb_limit_a': 1.5,
    'charge_limit_a': 0.5, 'boost_efficiency': 0.90, 'buck_efficiency': 0.90,
    'low_battery_v': 3.4, 'cutoff_v': 3.1, 'shutdown_timeout_s': 5.0,
}
BOUNDS = {
    'capacity_mah': (100, 20000), 'initial_soc': (0, 1), 'usb_limit_a': (0, 3),
    'charge_limit_a': (0, 2), 'boost_efficiency': (0.5, 1),
    'buck_efficiency': (0.5, 1), 'low_battery_v': (3, 4),
    'cutoff_v': (2.5, 3.5), 'shutdown_timeout_s': (0.1, 120),
}
USB_V = 5.0
USB_PATH_EFFICIENCY = 0.95
CHARGE_EFFICIENCY = 0.95
CELL_R_OHM = 0.12
MAX_DISCHARGE_A = 4.0
RECOVERY_HYSTERESIS_V = 0.2
MAX_DT_NS = 60_000_000_000
MAX_TIME_NS = (1 << 63) - 1
CELL_CURVE = ((0.0, 3.0), (0.05, 3.3), (0.15, 3.55), (0.5, 3.75),
              (0.9, 4.05), (1.0, 4.2))
BOOL_KEYS = {'usb_present', 'shutdown_ack', 'enable', 'fault_short', 'reset'}
LOAD_BOUNDS = {'host_load_w': (0, 20), 'radio_load_w': (0, 10),
               'sensor_load_w': (0, 5)}


def finite_number(name, value, bounds):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f'{name} must be a finite number')
    try:
        valid = math.isfinite(value) and bounds[0] <= value <= bounds[1]
    except OverflowError:
        valid = False
    if not valid:
        raise ValueError(f'{name} must be finite in [{bounds[0]}, {bounds[1]}]')
    return float(value)


def ocv(soc):
    for (s0, v0), (s1, v1) in zip(CELL_CURVE, CELL_CURVE[1:]):
        if soc <= s1:
            return v0 + (v1-v0) * (soc-s0) / (s1-s0)
    return CELL_CURVE[-1][1]


class PowerModel:
    def __init__(self):
        self.time_ns = 0
        self.config = None
        self.soc = None
        self.cutoff = False
        self.reason = 'none'
        self.request = False
        self.request_elapsed_s = 0.0

    def _validate(self, message):
        if not isinstance(message, dict) or set(message) != {'time_ns', 'dt_ns', 'inputs'}:
            raise ValueError('message must contain exactly time_ns, dt_ns, inputs')
        t, dt = message['time_ns'], message['dt_ns']
        if (type(t) is not int or type(dt) is not int or not 0 < dt <= MAX_DT_NS
                or not 0 < t <= MAX_TIME_NS or t != self.time_ns + dt):
            raise ValueError('time must be contiguous, strictly increasing integer ns; dt in (0, 60s]')
        inputs = message['inputs']
        if not isinstance(inputs, dict):
            raise ValueError('inputs must be an object')
        cfg = dict(self.config or DEFAULTS)
        for name, value in inputs.items():
            if name.startswith('config.') and name[7:] in BOUNDS:
                key = name[7:]
                number = finite_number(name, value, BOUNDS[key])
                if self.config is not None and number != self.config[key]:
                    raise ValueError(f'{name} is immutable after first step')
                cfg[key] = number
            elif name in BOOL_KEYS:
                if type(value) is not bool:
                    raise ValueError(f'{name} must be boolean')
            elif name in LOAD_BOUNDS:
                finite_number(name, value, LOAD_BOUNDS[name])
            else:
                raise ValueError(f'unknown input {name!r}')
        if cfg['cutoff_v'] >= cfg['low_battery_v']:
            raise ValueError('cutoff_v must be below low_battery_v')
        return t, dt / 1e9, inputs, cfg

    def _latch(self, reason):
        if not self.cutoff:
            self.cutoff, self.reason = True, reason

    def step(self, message):
        t, dt, inputs, cfg = self._validate(message)  # no state mutation on errors
        if self.config is None:
            self.config, self.soc = cfg, cfg['initial_soc']
        usb = inputs.get('usb_present', False)
        short = inputs.get('fault_short', False)
        enabled = inputs.get('enable', True)
        host = inputs.get('host_load_w', 0.0)
        radio = inputs.get('radio_load_w', 0.0)
        sensor = inputs.get('sensor_load_w', 0.0)
        requested = host + radio + sensor
        rail_load = host + radio + sensor / cfg['buck_efficiency']
        usb_budget = USB_V * cfg['usb_limit_a'] * USB_PATH_EFFICIENCY if usb else 0.0
        cell_ocv = ocv(self.soc)
        battery_needed = max(0.0, rail_load - usb_budget) / cfg['boost_efficiency']
        discriminant = cell_ocv**2 - 4*CELL_R_OHM*battery_needed
        overloaded = discriminant < 0
        discharge = ((2*battery_needed) / (cell_ocv + math.sqrt(discriminant))) if not overloaded else MAX_DISCHARGE_A + 1
        loaded_v = cell_ocv - discharge * CELL_R_OHM
        recovered = (cell_ocv >= cfg['low_battery_v'] + RECOVERY_HYSTERESIS_V and
                     ((usb and usb_budget >= rail_load and usb_budget > 0)
                     or (not overloaded and discharge <= MAX_DISCHARGE_A
                         and loaded_v >= cfg['low_battery_v'] + RECOVERY_HYSTERESIS_V)))
        if inputs.get('reset', False) and not short and recovered:
            self.cutoff, self.reason, self.request = False, 'none', False
            self.request_elapsed_s = 0.0
        if short:
            self._latch('short')
        battery_path = not usb or battery_needed > 0
        if enabled and battery_path and not self.cutoff:
            if loaded_v <= cfg['low_battery_v'] or overloaded:
                self.request = True
            if overloaded or discharge > MAX_DISCHARGE_A:
                self._latch('source_overload')
            elif loaded_v <= cfg['cutoff_v']:
                self._latch('undervoltage')
        # The proposed latch monitors the cell even with sufficient USB input.
        # Depleted-cell protection blocks output rails; USB may still charge it.
        if enabled and not self.cutoff and loaded_v <= cfg['cutoff_v']:
            self.request = True
            self._latch('undervoltage')
        if self.request and not self.cutoff:
            self.request_elapsed_s += dt
            if inputs.get('shutdown_ack', False):
                self._latch('shutdown_ack')
            elif self.request_elapsed_s >= cfg['shutdown_timeout_s']:
                self._latch('shutdown_timeout')

        powered = enabled and not self.cutoff
        delivered = requested if powered else 0.0
        bus_load = rail_load if powered else 0.0
        usb_to_load = min(usb_budget, bus_load)
        battery_power = max(0.0, bus_load-usb_to_load) / cfg['boost_efficiency']
        discharge = (2*battery_power / (cell_ocv + math.sqrt(cell_ocv**2-4*CELL_R_OHM*battery_power))) if battery_power else 0.0
        capacity_as = cfg['capacity_mah'] * 3.6
        # Zero SOC is caught by UV protection; remaining charge cannot go negative.
        discharge = min(discharge, self.soc * capacity_as / dt)
        battery_power = discharge * (cell_ocv - discharge*CELL_R_OHM)
        charge = 0.0
        if usb and not short and battery_power == 0:
            available = max(0.0, usb_budget-usb_to_load) * CHARGE_EFFICIENCY
            current_from_power = (2*available / (cell_ocv+math.sqrt(cell_ocv**2+4*CELL_R_OHM*available))) if available else 0.0
            hi = min(cfg['charge_limit_a'], current_from_power,
                     (1-self.soc)*capacity_as/dt)
            # CC/CV approximation: end-of-interval terminal voltage cannot exceed
            # 4.2 V, including the SOC change and series resistance. Bounded solve.
            lo = 0.0
            for _ in range(48):
                mid = (lo+hi)/2
                if ocv(min(1.0, self.soc+mid*dt/capacity_as)) + mid*CELL_R_OHM <= 4.2:
                    lo = mid
                else:
                    hi = mid
            charge = lo
        current = discharge-charge
        interval_v = cell_ocv + charge*CELL_R_OHM - discharge*CELL_R_OHM
        charge_power = charge * interval_v
        self.soc = min(1.0, max(0.0, self.soc-current*dt/capacity_as))
        final_v = ocv(self.soc) + charge*CELL_R_OHM-discharge*CELL_R_OHM
        if powered and battery_path and final_v <= cfg['low_battery_v']:
            self.request = True
        if powered and final_v <= cfg['cutoff_v']:
            self._latch('undervoltage')
        usb_bus_power = usb_to_load + charge_power / CHARGE_EFFICIENCY
        usb_power = usb_bus_power / USB_PATH_EFFICIENCY
        delivered_bus_power = usb_to_load + battery_power*cfg['boost_efficiency']
        # Delivered powers are interval averages, rails/protection are end state.
        delivered = delivered * delivered_bus_power / bus_load if bus_load else 0.0
        balance = usb_bus_power + battery_power*cfg['boost_efficiency'] - delivered_bus_power - charge_power/CHARGE_EFFICIENCY
        rails_on = enabled and not self.cutoff
        self.time_ns = t
        return {
            'soc': self.soc, 'battery_v': final_v,
            'battery_terminal_interval_v': interval_v,
            'battery_current_a': current, 'charge_current_a': charge,
            'battery_power_w': battery_power-charge_power,
            'usb_current_a': usb_power/USB_V,
            'usb_input_power_w': usb_power,
            'rail_5v': 5.0 if rails_on else 0.0,
            'rail_3v3': 3.3 if rails_on else 0.0,
            'requested_load_w': requested, 'rail_input_load_w': rail_load,
            'delivered_load_w': delivered, 'delivered_rail_input_w': delivered_bus_power,
            'shutdown_request': self.request, 'cutoff_latched': self.cutoff,
            'fault_reason': self.reason, 'power_balance_error_w': balance,
        }


def reject_constant(value):
    raise ValueError(f'nonfinite JSON constant {value}')


def serve(stdin=sys.stdin, stdout=sys.stdout):
    model = PowerModel()
    for line in stdin:
        message = json.loads(line, parse_constant=reject_constant)
        outputs = model.step(message)
        stdout.write(json.dumps({'outputs': outputs}, allow_nan=False, sort_keys=True)+'\n')
        stdout.flush()


if __name__ == '__main__':
    try:
        serve()
    except (ValueError, TypeError, OverflowError) as exc:
        print(f'power model error: {exc}', file=sys.stderr)
        sys.exit(1)
