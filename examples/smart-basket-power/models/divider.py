"""Real libngspice divider/RC check; no charger or switching-converter model."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(root / 'tools' / 'cosim'))
from labwired_ngspice import serve

serve((Path(__file__).resolve().parents[1] / 'divider-rc.cir').read_text(),
      sources={'battery_v': 'Vbattery'}, probes={'adc_v': 'v(adc)'})
