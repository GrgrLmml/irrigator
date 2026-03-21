use tracing::info;

pub struct Valve {
    #[cfg(target_os = "linux")]
    pin: rppal::gpio::OutputPin,
    #[cfg(not(target_os = "linux"))]
    open: bool,
}

impl Valve {
    #[cfg(target_os = "linux")]
    pub fn new(pin_number: u8) -> Result<Self, Box<dyn std::error::Error>> {
        let gpio = rppal::gpio::Gpio::new()?;
        let pin = gpio.get(pin_number)?.into_output_low();
        info!(pin = pin_number, "GPIO initialized, valve forced OFF");
        Ok(Self { pin })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new(pin_number: u8) -> Result<Self, Box<dyn std::error::Error>> {
        info!(pin = pin_number, "STUB: GPIO initialized (no real hardware)");
        Ok(Self { open: false })
    }

    #[cfg(target_os = "linux")]
    pub fn open(&mut self) {
        self.pin.set_high();
        info!("valve OPENED");
    }

    #[cfg(not(target_os = "linux"))]
    pub fn open(&mut self) {
        self.open = true;
        info!("STUB: valve OPENED");
    }

    #[cfg(target_os = "linux")]
    pub fn close(&mut self) {
        self.pin.set_low();
        info!("valve CLOSED");
    }

    #[cfg(not(target_os = "linux"))]
    pub fn close(&mut self) {
        self.open = false;
        info!("STUB: valve CLOSED");
    }
}

impl Drop for Valve {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        self.pin.set_low();
        #[cfg(not(target_os = "linux"))]
        {
            self.open = false;
        }
        info!("valve forced OFF on drop");
    }
}
