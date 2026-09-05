use crate::utils::lhm::{fetch, find_memory_load};

pub struct Ram;

impl Ram {
    pub fn new() -> Self {
        Ram
    }

    pub fn warn_missing(&self) {
        if self.get_usage().is_none() {
            crate::warning!("No supported RAM sensor was found");
        }
    }

    pub fn get_usage(&self) -> Option<u8> {
        let root = fetch()?;
        let pct = find_memory_load(&root)?;
        Some(pct.round().clamp(0.0, 100.0) as u8)
    }
}

impl Default for Ram {
    fn default() -> Self {
        Self::new()
    }
}
