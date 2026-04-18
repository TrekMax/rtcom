//! Terminal UI for rtcom.
//!
//! Hosts the `ratatui` render loop, the `SerialPane` (`vt100`-backed
//! terminal emulator), and the modal configuration menu. Subscribes
//! to [`rtcom_core::EventBus`] for serial data + system events;
//! publishes back `TxBytes`, `Command`, `MenuOpened`/`Closed`, etc.
#![forbid(unsafe_code)]

pub mod app;
pub mod layout;
pub mod serial_pane;
pub mod terminal;

pub use app::TuiApp;
pub use serial_pane::SerialPane;
