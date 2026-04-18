//! Core library for [rtcom](https://github.com/TrekMax/rtcom) — Rust Terminal Communication.
//!
//! At v0.1 this crate provides:
//!
//! - [`SerialDevice`] — async serial-port abstraction (Issue #2).
//! - [`SerialPortDevice`] — default backend built on [`tokio_serial`].
//! - [`SerialConfig`] and companion enums — the framing and flow parameters.
//! - [`Event`] and [`EventBus`] — cross-task event hub (Issue #5).
//! - [`Session`] — the orchestrator that drives serial I/O against the bus.
//! - [`Error`] / [`Result`] — the crate-wide error type.
//!
//! Later issues layer the event bus, session orchestrator, mappers, and
//! command state machine on top of this foundation. See `CLAUDE.md` §7 for
//! the full v0.1 plan.
//!
//! # Example
//!
//! ```no_run
//! use rtcom_core::{SerialConfig, SerialDevice, SerialPortDevice};
//! use tokio::io::AsyncWriteExt;
//!
//! # async fn run() -> rtcom_core::Result<()> {
//! let mut port = SerialPortDevice::open("/dev/ttyUSB0", SerialConfig::default())?;
//! port.write_all(b"hello\r\n").await?;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

pub mod command;
pub mod config;
pub mod device;
pub mod error;
pub mod event;
pub mod lock;
pub mod mapper;
pub mod session;

pub use command::{Command, CommandKeyParser, ParseOutput};
pub use config::{
    DataBits, FlowControl, ModemStatus, Parity, SerialConfig, StopBits, DEFAULT_READ_TIMEOUT,
};
pub use device::{SerialDevice, SerialPortDevice};
pub use error::{Error, Result};
pub use event::{Event, EventBus, DEFAULT_BUS_CAPACITY};
pub use lock::UucpLock;
pub use mapper::{LineEnding, LineEndingConfig, LineEndingMapper, Mapper};
pub use session::Session;
