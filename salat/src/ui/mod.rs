//! `render(frame, &UiModel, &Shell)` — the single UI entry point (§15).
//!
//! The tree is the §19 layout. Phase 5 declares the modules it
//! implements — theme, layout, model, widgets — and later phases add
//! the rest (input, shell, screens) as they are built.

pub mod layout;
pub mod model;
pub mod theme;
pub mod widgets;
