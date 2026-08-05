# Ping Pong

Two ESP32-C3 boards throw a ball back and forth over a serial wire. **Each
board has its own OLED** and draws the match live — both screens update as
the rally runs.

There is no host and no master. Each board only reacts to the other one, so if
you pull a wire the ball stops moving.

## Wiring

Cross the link (each board's TX goes to the other's RX) and connect the grounds.
If you skip the ground wire, neither board sees a single byte. That is the most
common way this fails.

Each board has its **own** SSD1306 on local GPIO4/5 (same pin numbers, separate
panels — do not share one OLED between both MCUs).

```
  Player A (+ OLED A)              Player B (+ OLED B)
  GPIO6 TX  -------------------->  GPIO7 RX
  GPIO7 RX  <--------------------  GPIO6 TX
  GND       ---------------------  GND

  OLED A SDA -> A GPIO4            OLED B SDA -> B GPIO4
  OLED A SCL -> A GPIO5            OLED B SCL -> B GPIO5
  OLED A VCC -> A 3V3              OLED B VCC -> B 3V3
  OLED A GND -> A GND              OLED B GND -> B GND
```

`Serial1` is the link between the boards. `Serial` (over USB) stays free for
messages you read on your laptop, so the two never collide.

## Flashing

Player A gets `server.ino`. Player B gets `screen.ino`.

Order does not matter. Player A serves right away and serves again after a
second of silence, so whichever board boots later just joins in.

## What you should see

Both OLEDs paint paddles + ball + rally count. Player A counts rallies over USB
as well. The ball moves one step per exchange, so the picture tracks the real
rally instead of running on its own timer.

Pull the link wire out. Player A prints `missed - rally ended at N` and serves
again. Plug it back in and the rally picks up.

## Libraries

None. The OLED is driven with plain `Wire` writes into a local framebuffer, so
there is nothing to install.

## Ideas

- Keep score: drop the ball if a return takes too long.
- Add a button so a person serves.
- Three boards in a ring passing the ball on.
- Send the rally count over USB and plot it.

## What has been tested

Both sketches compile on the hosted ESP32 toolchain.

`screen.ino` was run in the simulator with an OLED wired up. It boots through the
real C3 ROM, sets up the panel, draws, and prints its banner. Reading the panel
back shows 231 lit pixels on a 128x64 screen, so it really did draw rather than
just running the drawing code.

The two boards have not been run together in the hosted simulator, because that
path only boots one chip at a time. The wire itself is covered by
`crates/core/tests/world_esp32c3_pingpong.rs`.

In the **playground multi-chip** path, both MCUs run at once and the canvas
merges display framebuffers from every chip bridge so **both OLEDs paint while
either chip is selected**.

One thing worth knowing if you run this yourself: the C3's ROM boot uses up most
of a default step budget before your sketch starts. Give it a bigger budget or
the run stops during boot and reports an infinite loop that is not there.
