import json
import sys


CHANNEL_COUNT = 12


def disabled_channels(value):
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except json.JSONDecodeError:
            value = [value]

    if not isinstance(value, list):
        return set()

    disabled = set()
    for item in value:
        if isinstance(item, int):
            disabled.add(item)
        elif isinstance(item, str):
            if item.startswith("channel") and item[7:].isdigit():
                disabled.add(int(item[7:]))
            elif item.isdigit():
                disabled.add(int(item))
    return disabled


def step(inputs):
    disabled = disabled_channels(inputs.get("disabled_channels"))
    active = 0
    for index in range(CHANNEL_COUNT):
        if index in disabled:
            continue
        if inputs.get(f"channel{index}_enabled") is True:
            active += 1

    return {
        "v_out": float(active),
        "i_out": float(active),
        "active_channels": active,
    }


for line in sys.stdin:
    message = json.loads(line)
    outputs = step(message.get("inputs", {}))
    sys.stdout.write(json.dumps({"outputs": outputs}) + "\n")
    sys.stdout.flush()
