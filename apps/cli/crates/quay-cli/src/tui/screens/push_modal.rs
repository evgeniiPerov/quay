//! Push modal — tags / bump / remote form launched from Local `[U]`.
//!
//! This is a thin re-export shim.  The actual form state machine lives in
//! `create_push.rs`; the modal is invoked by switching to `Screen::CreatePush`
//! from the Local screen's key handlers:
//!   `[u]` — quick push (no form, immediate Patch bump)
//!   `[U]` — full push form (tags / bump / remote editable)

/// Re-export the create-push form builders so callers do not need to depend
/// directly on `create_push`.
pub use crate::tui::screens::create_push::build_create_form;
pub use crate::tui::screens::create_push::build_create_form_from_app;
pub use crate::tui::screens::create_push::build_push_existing_form;
