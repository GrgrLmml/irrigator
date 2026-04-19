use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tracing::info;

// Calibrated: 7L actual measured as 9.6L at 450 → 450 × 9.6/7 ≈ 617.
const PULSES_PER_LITER: f64 = 617.0;

pub struct FlowSensor {
    pulse_count: Arc<AtomicU64>,
    session_start_count: u64,
    session_started_at: Option<Instant>,
    #[cfg(target_os = "linux")]
    _pin: rppal::gpio::InputPin,
}

impl FlowSensor {
    #[cfg(target_os = "linux")]
    pub fn new(pin_number: u8) -> Result<Self, Box<dyn std::error::Error>> {
        let gpio = rppal::gpio::Gpio::new()?;
        let mut pin = gpio.get(pin_number)?.into_input_pullup();
        let pulse_count = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&pulse_count);
        pin.set_async_interrupt(rppal::gpio::Trigger::FallingEdge, None, move |_level| {
            counter.fetch_add(1, Ordering::Relaxed);
        })?;
        info!(pin = pin_number, "flow sensor initialized on GPIO");
        Ok(Self {
            pulse_count,
            session_start_count: 0,
            session_started_at: None,
            _pin: pin,
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new(pin_number: u8) -> Result<Self, Box<dyn std::error::Error>> {
        info!(pin = pin_number, "STUB: flow sensor initialized (no hardware)");
        Ok(Self {
            pulse_count: Arc::new(AtomicU64::new(0)),
            session_start_count: 0,
            session_started_at: None,
        })
    }

    /// Call when valve opens to reset session baseline.
    pub fn start_session(&mut self) {
        self.session_start_count = self.pulse_count.load(Ordering::Relaxed);
        self.session_started_at = Some(Instant::now());
        info!(baseline = self.session_start_count, "flow session started");
    }

    /// Get liters dispensed since start_session().
    pub fn session_liters(&self) -> f64 {
        let current = self.pulse_count.load(Ordering::Relaxed);
        let pulses = current.saturating_sub(self.session_start_count);
        pulses as f64 / PULSES_PER_LITER
    }

    /// Seconds elapsed since start_session(), or 0 if no session active.
    pub fn session_elapsed_secs(&self) -> u64 {
        self.session_started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }

    /// Current flow rate in L/min, averaged over the session so far.
    /// Returns 0 if session is too young (< 3s) to produce a meaningful rate.
    pub fn session_flow_lpm(&self) -> f64 {
        let secs = self.session_elapsed_secs();
        if secs < 3 {
            return 0.0;
        }
        self.session_liters() * 60.0 / (secs as f64)
    }
}
