//! Generation and decoded frames share the shell's single publication lock.
pub(super) struct Slot<T> {
    serial: u64,
    value: Option<T>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            serial: 0,
            value: None,
        }
    }
}

impl<T> Slot<T> {
    pub fn begin(&mut self) -> u64 {
        self.serial = self.serial.wrapping_add(1);
        self.value = None;
        self.serial
    }

    pub fn is_current(&self, serial: u64) -> bool {
        self.serial == serial
    }

    pub fn publish(&mut self, serial: u64, value: T) -> bool {
        if !self.is_current(serial) {
            return false;
        }
        self.value = Some(value);
        true
    }

    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::Slot;
    use std::sync::{Arc, Mutex, mpsc};

    #[test]
    fn completed_old_load_cannot_replace_stock_or_a_newer_load() {
        for replacement in [None, Some("new character")] {
            let slot = Arc::new(Mutex::new(Slot::default()));
            let old_serial = slot.lock().unwrap().begin();
            let (completed_tx, completed_rx) = mpsc::channel();
            let (publish_tx, publish_rx) = mpsc::channel();
            let old_load = {
                let slot = slot.clone();
                std::thread::spawn(move || {
                    // Decode A completes immediately before the UI selects B.
                    completed_tx.send(()).unwrap();
                    publish_rx.recv().unwrap();
                    slot.lock().unwrap().publish(old_serial, "old character")
                })
            };
            completed_rx.recv().unwrap();
            let current_serial = {
                let mut state = slot.lock().unwrap();
                let serial = state.begin();
                if let Some(value) = replacement {
                    assert!(state.publish(serial, value));
                }
                serial
            };
            publish_tx.send(()).unwrap();
            assert!(!old_load.join().unwrap());
            let state = slot.lock().unwrap();
            assert!(!state.is_current(old_serial));
            assert!(state.is_current(current_serial));
            assert_eq!(state.get().copied(), replacement);
        }
    }
}
