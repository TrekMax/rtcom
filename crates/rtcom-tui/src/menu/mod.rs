//! Configuration-menu dialogs.
//!
//! The entry point is [`RootMenu`], a seven-item top-level menu
//! pushed onto the [`crate::modal::ModalStack`] when the user opens
//! the configuration menu (`^A m`). Each item drills into a child
//! dialog (serial-port setup, line endings, modem control,
//! write/read profile, screen options). T15 replaces the last three
//! placeholder rows with real dialogs: [`ConfirmDialog`] (reused for
//! Write/Read profile) and [`ScreenOptionsDialog`].

pub mod confirm;
pub mod line_endings;
pub mod modem_control;
pub mod placeholder;
pub mod root;
pub mod screen_options;
pub mod serial_port;

pub use confirm::ConfirmDialog;
pub use line_endings::LineEndingsDialog;
pub use modem_control::ModemControlDialog;
pub use placeholder::PlaceholderDialog;
pub use root::RootMenu;
pub use screen_options::ScreenOptionsDialog;
pub use serial_port::SerialPortSetupDialog;
