//! Keeps the computer awake while downloads run (ADR-020).
//!
//! On Windows the wake request belongs to the thread that made it
//! (`SetThreadExecutionState`), so it must be taken and released on the same thread. The
//! async runtime moves work between threads, so one dedicated thread owns the
//! `keepawake` guard and receives on/off messages.

use std::sync::mpsc;

/// Handle to the sleep-prevention thread. Dropping it releases the guard and ends the
/// thread.
#[derive(Debug)]
pub struct SleepInhibitor {
    tx: mpsc::Sender<bool>,
}

impl SleepInhibitor {
    /// Starts the thread (it sleeps on a channel; no CPU while nothing changes).
    ///
    /// # Errors
    /// If the OS cannot create a thread.
    pub fn start() -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("persivio-keepawake".to_owned())
            .spawn(move || {
                let mut guard = None;
                while let Ok(on) = rx.recv() {
                    if on && guard.is_none() {
                        guard = keepawake::Builder::default()
                            .idle(true)
                            .reason("Downloads in progress")
                            .app_name("Persivio")
                            .app_reverse_domain("io.github.echosns.persivio")
                            .create()
                            .inspect_err(|e| tracing::warn!("cannot prevent sleep: {e}"))
                            .ok();
                    } else if !on {
                        guard = None;
                    }
                }
            })?;
        Ok(Self { tx })
    }

    /// Prevents (`true`) or allows (`false`) idle sleep. Repeated calls are cheap.
    pub fn set(&self, prevent: bool) {
        let _ = self.tx.send(prevent);
    }
}
