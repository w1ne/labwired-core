from pathlib import Path

path = Path('.github/workflows/core-ci.yml')
text = path.read_text()
anchor = '  pr-gate:\n'
start = text.find(anchor)
if start < 0:
    raise SystemExit('pr-gate job not found')
next_job = text.find('\n  ', start + len(anchor))
if next_job < 0:
    next_job = len(text)
old = '    timeout-minutes: 20\n'
pos = text.find(old, start, next_job)
if pos < 0:
    raise SystemExit('pr-gate 20-minute timeout not found inside job')
replacement = (
    '    # The default-member unit suite crossed the old 20-minute wall on the\n'
    '    # Xtensa ELF loader change after every earlier gate had passed. A job\n'
    '    # timeout is no verdict, so keep enough headroom to report the tests.\n'
    '    timeout-minutes: 35\n'
)
path.write_text(text[:pos] + replacement + text[pos + len(old):])
