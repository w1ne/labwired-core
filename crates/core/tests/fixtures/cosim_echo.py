import json
import sys


for line in sys.stdin:
    msg = json.loads(line)
    sys.stdout.write(
        json.dumps(
            {
                "outputs": {
                    "ack": True,
                    "time_ns": msg["time_ns"],
                }
            }
        )
        + "\n"
    )
    sys.stdout.flush()
