//! ILI9341 command/scanline adapter — streams a 160×120 frame as 320×240 2× NN.

use crate::render::{HEIGHT, WIDTH};

/// Sink that accepts the internal render target and presents it on the panel.
pub trait PixelSink {
    type Error;
    fn init(&mut self) -> Result<(), Self::Error>;
    fn present_2x(&mut self, frame: &[u16; WIDTH * HEIGHT]) -> Result<(), Self::Error>;
}

// ILI9341 commands
const SWRESET: u8 = 0x01;
const SLPOUT: u8 = 0x11;
const DISPON: u8 = 0x29;
const CASET: u8 = 0x2A;
const PASET: u8 = 0x2B;
const RAMWR: u8 = 0x2C;
const MADCTL: u8 = 0x36;
const COLMOD: u8 = 0x3A;

/// Low-level bus operations the concrete SPI+GPIO driver must provide.
pub trait Ili9341Bus {
    type Error;
    fn write_command(&mut self, cmd: u8) -> Result<(), Self::Error>;
    fn write_data(&mut self, data: &[u8]) -> Result<(), Self::Error>;
    fn delay_ms(&mut self, ms: u32);
}

/// Generic ILI9341 presenter over any `Ili9341Bus`.
pub struct Ili9341Display<B: Ili9341Bus> {
    bus: B,
}

impl<B: Ili9341Bus> Ili9341Display<B> {
    pub fn new(bus: B) -> Self {
        Self { bus }
    }

    pub fn into_inner(self) -> B {
        self.bus
    }

    fn cmd(&mut self, c: u8) -> Result<(), B::Error> {
        self.bus.write_command(c)
    }

    fn cmd_data(&mut self, c: u8, data: &[u8]) -> Result<(), B::Error> {
        self.bus.write_command(c)?;
        self.bus.write_data(data)
    }

    fn set_window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) -> Result<(), B::Error> {
        self.cmd_data(
            CASET,
            &[
                (x0 >> 8) as u8,
                x0 as u8,
                (x1 >> 8) as u8,
                x1 as u8,
            ],
        )?;
        self.cmd_data(
            PASET,
            &[
                (y0 >> 8) as u8,
                y0 as u8,
                (y1 >> 8) as u8,
                y1 as u8,
            ],
        )
    }
}

impl<B: Ili9341Bus> PixelSink for Ili9341Display<B> {
    type Error = B::Error;

    fn init(&mut self) -> Result<(), Self::Error> {
        self.cmd(SWRESET)?;
        self.bus.delay_ms(5);
        self.cmd(SLPOUT)?;
        self.bus.delay_ms(5);
        // 16-bit/pixel RGB565
        self.cmd_data(COLMOD, &[0x55])?;
        // MX=0 MY=0 MV=0 BGR=0 — portrait 240×320 native; we write 320×240
        // with MADCTL MV bit so columns map to the long edge.
        // MADCTL: MY MX MV ML BGR MH 0 0 — set MV|MX for landscape 320×240.
        self.cmd_data(MADCTL, &[0x28])?;
        self.cmd(DISPON)?;
        self.bus.delay_ms(5);
        // Full-panel window 0..319 × 0..239
        self.set_window(0, 0, 319, 239)?;
        Ok(())
    }

    fn present_2x(&mut self, frame: &[u16; WIDTH * HEIGHT]) -> Result<(), Self::Error> {
        // Ensure the address window covers the full 320×240 panel.
        self.set_window(0, 0, 319, 239)?;
        self.cmd(RAMWR)?;

        // Stream without allocating a 320×240 buffer: each source pixel is
        // emitted twice horizontally; each source row is emitted twice.
        // RGB565 is sent big-endian (high byte first) per ILI9341.
        let mut pair = [0u8; 4];
        for row in 0..HEIGHT {
            for _dup_row in 0..2 {
                for col in 0..WIDTH {
                    let px = frame[row * WIDTH + col];
                    let hi = (px >> 8) as u8;
                    let lo = px as u8;
                    pair[0] = hi;
                    pair[1] = lo;
                    pair[2] = hi;
                    pair[3] = lo;
                    self.bus.write_data(&pair)?;
                }
            }
        }
        Ok(())
    }
}

/// Solid-colour diagnostic frame (used when init fails or for boot flash).
pub fn fill_frame(frame: &mut [u16; WIDTH * HEIGHT], color: u16) {
    for p in frame.iter_mut() {
        *p = color;
    }
}

/// Pure-logic helper tests for 2× expansion math (no SPI).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::rgb565;

    struct RecordingBus {
        commands: heapless_cmds::CmdLog,
        pixels: heapless_cmds::PixelLog,
        in_ramwr: bool,
    }

    // Avoid a heapless dependency: fixed-cap logs for the test.
    mod heapless_cmds {
        pub struct CmdLog {
            pub data: [u8; 32],
            pub len: usize,
        }
        impl CmdLog {
            pub fn new() -> Self {
                Self {
                    data: [0; 32],
                    len: 0,
                }
            }
            pub fn push(&mut self, c: u8) {
                if self.len < self.data.len() {
                    self.data[self.len] = c;
                    self.len += 1;
                }
            }
            pub fn as_slice(&self) -> &[u8] {
                &self.data[..self.len]
            }
        }
        pub struct PixelLog {
            pub count: usize,
        }
        impl PixelLog {
            pub fn new() -> Self {
                Self { count: 0 }
            }
        }
    }

    impl Ili9341Bus for RecordingBus {
        type Error = ();
        fn write_command(&mut self, cmd: u8) -> Result<(), ()> {
            self.commands.push(cmd);
            self.in_ramwr = cmd == RAMWR;
            Ok(())
        }
        fn write_data(&mut self, data: &[u8]) -> Result<(), ()> {
            if self.in_ramwr {
                // Each pixel is 2 bytes.
                self.pixels.count += data.len() / 2;
            }
            Ok(())
        }
        fn delay_ms(&mut self, _ms: u32) {}
    }

    #[test]
    fn init_sends_required_commands() {
        let bus = RecordingBus {
            commands: heapless_cmds::CmdLog::new(),
            pixels: heapless_cmds::PixelLog::new(),
            in_ramwr: false,
        };
        let mut d = Ili9341Display::new(bus);
        d.init().unwrap();
        let cmds = d.bus.commands.as_slice();
        assert!(cmds.contains(&SWRESET));
        assert!(cmds.contains(&SLPOUT));
        assert!(cmds.contains(&COLMOD));
        assert!(cmds.contains(&MADCTL));
        assert!(cmds.contains(&DISPON));
        assert!(cmds.contains(&CASET));
        assert!(cmds.contains(&PASET));
    }

    #[test]
    fn present_2x_emits_full_panel_pixels() {
        let bus = RecordingBus {
            commands: heapless_cmds::CmdLog::new(),
            pixels: heapless_cmds::PixelLog::new(),
            in_ramwr: false,
        };
        let mut d = Ili9341Display::new(bus);
        d.init().unwrap();
        let mut frame = [0u16; WIDTH * HEIGHT];
        fill_frame(&mut frame, rgb565(255, 0, 0));
        d.present_2x(&frame).unwrap();
        // 160×120 source → 320×240 = 76800 pixels.
        assert_eq!(d.bus.pixels.count, 320 * 240);
    }
}
