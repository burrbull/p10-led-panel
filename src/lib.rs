#![no_std]

use core::marker::PhantomData;

use embedded_graphics_core::{
    geometry::{Dimensions, Size},
    Pixel,
};
use embedded_hal::{
    digital::{OutputPin, PinState},
    pwm::SetDutyCycle,
    spi::SpiDevice,
};

#[derive(Clone, Copy, Debug)]
pub enum Error {
    Spi,
    Pwm,
    Digital,
}

pub struct Blocking;
#[cfg(feature = "async")]
pub struct Async;

pub struct P10Led<
    SPI,
    PWM: SetDutyCycle,
    A: OutputPin,
    B: OutputPin,
    L: OutputPin,
    const PX: usize = 1,
    const PY: usize = 1,
    MODE = Blocking,
> {
    spi: SPI,
    pwm: PWM,
    pin_a: A,
    pin_b: B,
    latch: L,
    brightness: u16,
    bitmap: [u8; 256], // TODO: size ???
    cache: [u8; 64],   // TODO: size ???
    scan_row: u8,
    _mode: PhantomData<MODE>,
}

impl<
        SPI,
        PWM: SetDutyCycle,
        A: OutputPin,
        B: OutputPin,
        L: OutputPin,
        const PX: usize,
        const PY: usize,
        MODE,
    > P10Led<SPI, PWM, A, B, L, PX, PY, MODE>
{
    pub const PANEL_WIDTH: usize = 32;
    pub const PANEL_HEIGHT: usize = 16;
    pub const WIDTH: usize = PX * Self::PANEL_WIDTH;
    pub const HEIGHT: usize = PY * Self::PANEL_HEIGHT;
    pub const HEIGHT_IN_PANELS: usize = PY;

    pub const fn row_width_bytes() -> usize {
        if Self::WIDTH % 8 == 0 {
            Self::WIDTH / 8
        } else {
            Self::WIDTH / 8 + 1
        }
    }
    pub const fn unified_width_bytes() -> usize {
        Self::row_width_bytes() * Self::HEIGHT_IN_PANELS
    }

    const fn pixel_to_bitmap_index(x: usize, y: usize) -> usize {
        let panel = (x / Self::PANEL_WIDTH)
            + ((Self::WIDTH / Self::PANEL_WIDTH) * (y / Self::PANEL_HEIGHT));
        let x = (x % Self::PANEL_WIDTH) + (panel * Self::PANEL_WIDTH);
        let y = y % Self::PANEL_HEIGHT;
        x / 8 + y * Self::unified_width_bytes()
    }

    const fn pixel_to_bitmask(x: usize) -> u8 {
        1 << (8 - x % 8)
    }

    pub fn set_brightness(&mut self, brightness: u16) -> Result<(), Error> {
        self.brightness = brightness;
        self.pwm
            .set_duty_cycle_fraction(brightness, 65535)
            .map_err(|_| Error::Pwm)
    }

    fn fill_cache(&mut self) {
        let rowsize = Self::unified_width_bytes();
        let scan_row = self.scan_row as usize;
        {
            for (chunk, (((&r0, &r4), &r8), &r12)) in self.cache.chunks_exact_mut(4).zip(
                self.bitmap
                    .iter()
                    .skip((scan_row + 0) * rowsize)
                    .take(rowsize)
                    .zip(
                        self.bitmap
                            .iter()
                            .skip((scan_row + 4) * rowsize)
                            .take(rowsize),
                    )
                    .zip(
                        self.bitmap
                            .iter()
                            .skip((scan_row + 8) * rowsize)
                            .take(rowsize),
                    )
                    .zip(
                        self.bitmap
                            .iter()
                            .skip((scan_row + 12) * rowsize)
                            .take(rowsize),
                    ),
            ) {
                chunk.copy_from_slice(&[r0, r4, r8, r12]);
            }
        }
    }

    fn next_row(&mut self) -> Result<(), Error> {
        // Disable PWM
        self.pwm
            .set_duty_cycle_fully_off()
            .map_err(|_| Error::Pwm)?;
        // Latch
        self.latch.set_high().map_err(|_| Error::Digital)?; // Latch DMD shift register output
        self.latch.set_low().map_err(|_| Error::Digital)?; // (Deliberately left as digitalWrite to ensure decent latching time)

        // Digital outputs A, B are a 2-bit selector output, set from the scan_row variable (loops over 0-3),
        // that determines which set of interleaved rows we are outputting during this pass.
        // BA 0 (00) = 1,5,9,13
        // BA 1 (01) = 2,6,10,14
        // BA 2 (10) = 3,7,11,15
        // BA 3 (11) = 4,8,12,16
        self.pin_a
            .set_state(PinState::from(self.scan_row & 0b01 != 0))
            .map_err(|_| Error::Digital)?;
        self.pin_b
            .set_state(PinState::from(self.scan_row & 0b10 != 0))
            .map_err(|_| Error::Digital)?;
        self.scan_row = (self.scan_row + 1) % 4;

        // Reenable PWM
        self.pwm
            .set_duty_cycle_fraction(self.brightness, 65535)
            .map_err(|_| Error::Pwm)?;

        Ok(())
    }
}

impl<
        SPI: SpiDevice,
        PWM: SetDutyCycle,
        A: OutputPin,
        B: OutputPin,
        L: OutputPin,
        const PX: usize,
        const PY: usize,
    > P10Led<SPI, PWM, A, B, L, PX, PY, Blocking>
{
    pub fn new(
        spi: SPI,
        mut pwm: PWM,
        pin_a: A,
        pin_b: B,
        latch: L,
        brightness: u16,
    ) -> Result<Self, Error> {
        pwm.set_duty_cycle_fraction(brightness, 65535)
            .map_err(|_| Error::Pwm)?;
        Ok(Self {
            spi,
            pwm,
            pin_a,
            pin_b,
            latch,
            brightness,
            bitmap: [0xff; 256],
            cache: [0xff; 64],
            scan_row: 0,
            _mode: PhantomData,
        })
    }

    #[cfg(feature = "async")]
    pub fn asynch(self) -> P10Led<SPI, PWM, A, B, L, PX, PY, Async> {
        P10Led {
            spi: self.spi,
            pwm: self.pwm,
            pin_a: self.pin_a,
            pin_b: self.pin_b,
            latch: self.latch,
            brightness: self.brightness,
            bitmap: self.bitmap,
            cache: self.cache,
            scan_row: self.scan_row,
            _mode: PhantomData,
        }
    }

    /// Method to flush framebuffer to display. This method needs to be called everytime a new framebuffer is created,
    /// otherwise the frame will not appear on the screen.
    pub fn flush(&mut self) -> Result<(), Error> {
        for _ in 0..4 {
            self.fill_cache();
            self.spi.write(&self.cache).map_err(|_| Error::Spi)?;

            self.next_row()?;
        }
        Ok(())
    }
}

#[cfg(feature = "async")]
impl<
        SPI: embedded_hal_async::spi::SpiDevice,
        PWM: SetDutyCycle,
        A: OutputPin,
        B: OutputPin,
        L: OutputPin,
        const PX: usize,
        const PY: usize,
    > P10Led<SPI, PWM, A, B, L, PX, PY, Async>
{
    pub fn blocking(self) -> P10Led<SPI, PWM, A, B, L, PX, PY, Blocking> {
        P10Led {
            spi: self.spi,
            pwm: self.pwm,
            pin_a: self.pin_a,
            pin_b: self.pin_b,
            latch: self.latch,
            brightness: self.brightness,
            bitmap: self.bitmap,
            cache: self.cache,
            scan_row: self.scan_row,
            _mode: PhantomData,
        }
    }

    /// Method to flush framebuffer to display. This method needs to be called everytime a new framebuffer is created,
    /// otherwise the frame will not appear on the screen.
    pub async fn flush(&mut self) -> Result<(), Error> {
        for _ in 0..4 {
            self.fill_cache();
            self.spi.write(&self.cache).await.map_err(|_| Error::Spi)?;

            self.next_row()?;
        }
        Ok(())
    }
}

impl<
        SPI,
        PWM: SetDutyCycle,
        A: OutputPin,
        B: OutputPin,
        L: OutputPin,
        const PX: usize,
        const PY: usize,
        MODE,
    > embedded_graphics_core::draw_target::DrawTarget for P10Led<SPI, PWM, A, B, L, PX, PY, MODE>
{
    type Color = embedded_graphics_core::pixelcolor::BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let bb = self.bounding_box();
        for Pixel(pos, color) in pixels
            .into_iter()
            .filter(|Pixel(pos, _color)| bb.contains(*pos))
        {
            let byte_idx = Self::pixel_to_bitmap_index(pos.x as _, pos.y as _);
            let bit = Self::pixel_to_bitmask(pos.x as _);
            if color.is_on() {
                self.bitmap[byte_idx] &= !bit; // and with the inverse of the bit - so
            } else {
                self.bitmap[byte_idx] |= bit; // set bit (which turns it off)
            }
        }
        Ok(())
    }
}
impl<
        SPI,
        PWM: SetDutyCycle,
        A: OutputPin,
        B: OutputPin,
        L: OutputPin,
        const PX: usize,
        const PY: usize,
        MODE,
    > embedded_graphics_core::geometry::OriginDimensions
    for P10Led<SPI, PWM, A, B, L, PX, PY, MODE>
{
    fn size(&self) -> Size {
        Size::new(Self::WIDTH as _, Self::HEIGHT as _)
    }
}
