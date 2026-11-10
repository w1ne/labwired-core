# FAULTMASK YAML parse failure

The first FAULTMASK target-only attempt built the CLI and firmware, but the
smoke runner rejected the new final `memory_value` assertion before firmware
execution. Its nested fields were indented at the assertion level instead of
inside `memory_value`. The failed metrics and logs are preserved in `metrics/`.

The runner was also invoked with `python3`, whose scoreboard-generator
environment lacks PyYAML. Use `/usr/bin/python3` for the next target-only run;
the failed run did not update the scoreboard.
