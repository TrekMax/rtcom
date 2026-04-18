//! Configuration-menu dialogs.
//!
//! The entry point is [`RootMenu`], a seven-item top-level menu
//! pushed onto the [`crate::modal::ModalStack`] when the user opens
//! the configuration menu (`^A m`). Each item drills into a child
//! dialog (serial-port setup, line endings, ...). For v0.2 task 11
//! every child is a [`PlaceholderDialog`]; later tasks (T12+) replace
//! them with real dialogs.

pub mod line_endings;
pub mod placeholder;
pub mod root;
pub mod serial_port;

pub use line_endings::LineEndingsDialog;
pub use placeholder::PlaceholderDialog;
pub use root::RootMenu;
pub use serial_port::SerialPortSetupDialog;
