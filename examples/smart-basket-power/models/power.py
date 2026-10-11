"""Repository-local entrypoint for the native external_process adapter."""
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'tools' / 'cosim'))
from smart_basket_power import serve

if __name__ == '__main__':
    try:
        serve()
    except (ValueError, TypeError, OverflowError) as exc:
        print(f'power model error: {exc}', file=sys.stderr)
        sys.exit(1)
