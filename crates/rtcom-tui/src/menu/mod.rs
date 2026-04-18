//! Configuration-menu dialogs.
//!
//! The entry point is [`RootMenu`], a seven-item top-level menu
//! pushed onto the [`crate::modal::ModalStack`] when the user opens
//! the configuration menu (`^A m`). Each item drills into a child
//! dialog (serial-port setup, line endings, modem control, ...). T12
//! replaced the first row with a real [`SerialPortSetupDialog`]; T13
//! did the same for "Line endings" via [`LineEndingsDialog`]; T14
//! does the same for "Modem control" via [`ModemControlDialog`]; the
//! remaining rows are still [`PlaceholderDialog`]s until T15+ lands.

pub mod line_endings;
pub mod modem_control;
pub mod placeholder;
pub mod root;
pub mod serial_port;

pub use line_endings::LineEndingsDialog;
pub use modem_control::ModemControlDialog;
pub use placeholder::PlaceholderDialog;
pub use root::RootMenu;
pub use serial_port::SerialPortSetupDialog;
