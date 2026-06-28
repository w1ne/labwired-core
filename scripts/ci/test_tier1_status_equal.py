import subprocess, sys, json
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "tier1_status_equal.py"

def _write(tmp, name, data):
    p = tmp / name
    p.write_text(json.dumps(data))
    return str(p)

def _run(a, b):
    return subprocess.run([sys.executable, str(SCRIPT), a, b]).returncode

def test_equal_when_only_run_url_differs(tmp_path):
    a = _write(tmp_path, "a.json", {"esp32": {"adc": {"status": "pass", "run_url": "https://x/1"}}})
    b = _write(tmp_path, "b.json", {"esp32": {"adc": {"status": "pass", "run_url": "https://x/2"}}})
    assert _run(a, b) == 0

def test_differs_when_status_changes(tmp_path):
    a = _write(tmp_path, "a.json", {"esp32": {"adc": {"status": "pass"}}})
    b = _write(tmp_path, "b.json", {"esp32": {"adc": {"status": "partial"}}})
    assert _run(a, b) == 1

def test_differs_when_cell_added(tmp_path):
    a = _write(tmp_path, "a.json", {"esp32": {"adc": {"status": "pass"}}})
    b = _write(tmp_path, "b.json", {"esp32": {"adc": {"status": "pass"}, "spi": {"status": "pass"}}})
    assert _run(a, b) == 1
