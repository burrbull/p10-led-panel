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

pub struct P10Led<
    SPI: SpiDevice,
    PWM: SetDutyCycle,
    A: OutputPin,
    B: OutputPin,
    E: OutputPin,
    L: OutputPin,
    const COUNT: usize = 1,
> {
    spi: SPI,
    pwm: PWM,
    pin_a: A,
    pin_b: B,
    enable: E,
    latch: L,
    duty: u16,
    cache: [[u8; 16]; 4],
}
impl<SPI: SpiDevice, PWM: SetDutyCycle, A: OutputPin, B: OutputPin, E: OutputPin, L: OutputPin, const COUNT: usize>
    P10Led<SPI, PWM, A, B, E, L, COUNT>
{
    pub fn new(
        spi: SPI,
        mut pwm: PWM,
        pin_a: A,
        pin_b: B,
        enable: E,
        latch: L,
        duty: u16,
    ) -> Result<Self, Error> {
        pwm.set_duty_cycle_fraction(duty, 65535)
            .map_err(|_| Error::Pwm)?;
        Ok(Self {
            spi,
            pwm,
            pin_a,
            pin_b,
            enable,
            latch,
            duty,
            cache: [[0; 16]; 4],
        })
    }

    pub fn write_data(&mut self, data: &[u8]) -> Result<(), Error> {
        assert_eq!(data.len(), COUNT*512);
        let mut o = 0;
        for _ in 0..COUNT*32 {
            // Write cache
            self.spi.write(&self.cache[o]).map_err(|_| Error::Spi)?;

            // Disable PWM
            self.pwm
                .set_duty_cycle_fully_off()
                .map_err(|_| Error::Pwm)?;
            self.enable.set_low().map_err(|_| Error::Digital)?;

            // Latch
            self.latch.set_high().map_err(|_| Error::Digital)?;
            self.latch.set_low().map_err(|_| Error::Digital)?;

            // Row
            self.pin_a
                .set_state(PinState::from(o & 1 != 0))
                .map_err(|_| Error::Digital)?;
            self.pin_b
                .set_state(PinState::from(o & 2 != 0))
                .map_err(|_| Error::Digital)?;

            // Reenable PWM
            self.pwm
                .set_duty_cycle_fraction(self.duty, 65535)
                .map_err(|_| Error::Pwm)?;

            // Cache data
            for i in 0..16 {
                self.cache[o][i] = !data[((3 - i) % 4) * 16 + o * 4 + (i / 4)];
            }

            o = (o + 1) % 4;

            //await aio.sleep(0)
        }

        Ok(())
    }
}
