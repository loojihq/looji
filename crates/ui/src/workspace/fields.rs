//! The field pieces used by the dialogs and the detail panel. They now live in `components`, so every
//! screen draws the same controls. This module re-exports them so existing imports keep working.

pub use crate::components::{chip, field, member_mark, segment};
