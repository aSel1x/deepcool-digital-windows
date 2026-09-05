//! Системные вентиляторы (не CPU и не помпа).

use super::to_u16;
use crate::utils::lhm::{fetch, find_sys_fan};

pub struct SysFan;

impl SysFan {
    pub fn new() -> Self {
        SysFan
    }

    /// Обороты системного вентилятора #N.
    pub fn get_rpm(&self, index: usize) -> Option<u16> {
        let root = fetch()?;
        find_sys_fan(&root, index).map(to_u16)
    }
}

impl Default for SysFan {
    fn default() -> Self {
        Self::new()
    }
}
