use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Ok;
use esp_idf_svc::hal::{
    gpio::{Input, InputPin, OutputPin, PinDriver, Pull, InterruptType},
    peripheral::Peripheral,
};

pub type ButtonCallback = Arc<dyn Fn() + Send + Sync>;

pub struct InputButton<P: InputPin + OutputPin> {
    driver: Arc<Mutex<PinDriver<'static, P, Input>>>,
    pressed: Arc<AtomicBool>,
    last_press_ms: Arc<AtomicUsize>,
    debounce_ms: usize,
    callback: Option<ButtonCallback>,
}

impl<P: InputPin + OutputPin> InputButton<P> {
    pub fn new(pin: impl Peripheral<P = P> + 'static, debounce_ms: usize) -> anyhow::Result<Self> {
        let mut driver = PinDriver::input(pin)?;
        driver.set_pull(Pull::Up)?;
        driver.set_interrupt_type(InterruptType::NegEdge)?;

        Ok(Self {
            driver: Arc::new(Mutex::new(driver)),
            pressed: Arc::new(AtomicBool::new(false)),
            last_press_ms: Arc::new(AtomicUsize::new(0)),
            debounce_ms,
            callback: None,
        })
    }

    /// Set a callback to be invoked when the button is pressed (in ISR context).
    /// Callback should be fast and non-blocking.
    pub fn set_callback<F: Fn() + Send + Sync + 'static>(&mut self, callback: F) -> anyhow::Result<()> {
        self.callback = Some(Arc::new(callback));
        let pressed = self.pressed.clone();
        let last_press = self.last_press_ms.clone();
        let debounce = self.debounce_ms;
        let callback = self.callback.clone();
        let driver = self.driver.clone();
        let mut locked_driver = self.driver.lock().unwrap();
        unsafe {
            locked_driver.subscribe(move || {
                let now_ms = (esp_idf_svc::sys::esp_timer_get_time() / 1000) as usize;
                let last = last_press.load(Ordering::SeqCst);

                // Check if enough time has passed since last accepted press
                if now_ms.saturating_sub(last) >= debounce {
                    // Update timestamp immediately to prevent re-triggering during debounce window
                    last_press.store(now_ms, Ordering::SeqCst);
                    pressed.store(true, Ordering::SeqCst);

                    if let Some(cb) = &callback {
                        cb();
                    }
                } else {
                    // Still in debounce window, update timestamp to extend the window
                    last_press.store(now_ms, Ordering::SeqCst);
                }
                let mut driver = driver.lock().unwrap();
                driver.enable_interrupt().unwrap();
            })?;
        }
        locked_driver.enable_interrupt()?;
        Ok(())
    }

    /// Check if button was pressed and reset the flag.
    pub fn is_pressed(&self) -> bool {
        self.pressed.swap(false, Ordering::Relaxed)
    }

    /// Get current button state (true = pressed / low).
    pub fn is_active(&self) -> bool {
        let driver = self.driver.lock().unwrap();
        driver.is_low()
    }
}
