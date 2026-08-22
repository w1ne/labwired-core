# LabWired README demo — regenerate with:
#
#   brew install vhs
#   cargo build --release -p labwired-cli
#   vhs docs/assets/demo.tape
#
# This records the REAL quickstart from README.md against a REAL binary. It is
# not a mockup and nothing here is staged: the nRF54L15 boot tokens, the
# register bases, and the verdict line are whatever the simulator actually
# printed at record time. If the product regresses, re-recording shows it.
#
# The second run is deliberate. A demo that only ever shows green proves the
# tool runs, not that it CHECKS anything — so the tape also mutates one
# assertion and shows the gate failing with a non-zero exit.

Output docs/assets/labwired-demo.gif

Require labwired

Set Shell zsh
Set FontSize 17
Set Width 1100
Set Height 430
Set Padding 24
Set Margin 0
Set TypingSpeed 32ms
Set Theme { "name": "labwired", "background": "#14151a", "foreground": "#e6e8eb", "cursor": "#4d9fff", "selection": "#2a3550", "black": "#14151a", "red": "#ff6b6b", "green": "#51cf66", "yellow": "#ffd43b", "blue": "#4d9fff", "magenta": "#da77f2", "cyan": "#3bc9db", "white": "#e6e8eb", "brightBlack": "#6b7280", "brightRed": "#ff8787", "brightGreen": "#69db7c", "brightYellow": "#ffe066", "brightBlue": "#74c0fc", "brightMagenta": "#e599f7", "brightCyan": "#66d9e8", "brightWhite": "#f8f9fa" }

# Quiet the shell and the tracing INFO lines so the recording shows the
# product, not the harness. RUST_LOG is honoured for real now.
Hide
Type "export PS1='$ ' RUST_LOG=warn && clear"
Enter
Show

Sleep 800ms

Type "labwired test --script examples/nrf54l15-dk/io-smoke.yaml"
Sleep 400ms
Enter
Sleep 3s

Type "echo $?"
Sleep 300ms
Enter
Sleep 1.5s

# Now break one assertion and prove the gate actually gates.
#
# The mutant lives NEXT TO the original on purpose: a test script resolves
# `inputs.firmware` and `system` relative to its own directory, so a copy in
# /tmp would fail on a missing file rather than on the assertion, which would
# demonstrate nothing.
Hide
Type "sed 's/nRF54L15 boot OK/A LINE THE FIRMWARE NEVER PRINTS/' examples/nrf54l15-dk/io-smoke.yaml > examples/nrf54l15-dk/broken.yaml && clear"
Enter
Show

Sleep 600ms

Type "labwired test --script examples/nrf54l15-dk/broken.yaml"
Sleep 400ms
Enter
Sleep 3s

Type "echo $?"
Sleep 300ms
Enter
Sleep 1.6s

Hide
Type "rm -f examples/nrf54l15-dk/broken.yaml"
Enter
Show
