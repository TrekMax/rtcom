//! Terminal UI for rtcom.
//!
//! Hosts the `ratatui` render loop, the `SerialPane` (`vt100`-backed
//! terminal emulator), and the modal configuration menu. Subscribes
//! to [`rtcom_core::EventBus`] for serial data + system events;
//! publishes back `TxBytes`, `Command`, `MenuOpened`/`Closed`, etc.
#![forbid(unsafe_code)]

pub mod app;
pub mod input;
pub mod layout;
pub mod modal;
pub mod serial_pane;
pub mod terminal;

pub use app::TuiApp;
pub use input::Dispatch;
pub use modal::{Dialog, DialogAction, DialogOutcome, ModalStack};
pub use serial_pane::SerialPane;
