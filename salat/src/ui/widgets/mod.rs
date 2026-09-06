//! The §15.5 widget inventory — each a render function or a small
//! struct, each independently snapshot-tested, each under ~60 lines.
//! The widgets own shape only; every glyph, word and colour comes
//! from [`crate::ui::theme`], so a mode change is one place.

pub mod badge;
pub mod bar;
pub mod bigtext;
pub mod hr;
pub mod kbd_hint;
pub mod panel;
pub mod prayer_table;
pub mod status_dot;
