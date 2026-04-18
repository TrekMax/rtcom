//! Cross-task event bus for `rtcom-core`.
//!
//! Every meaningful thing that happens in a [`Session`](crate::Session) — a
//! chunk of bytes from the wire, a chunk pending transmission, a config
//! change, a fatal error — flows through the [`EventBus`] as an [`Event`].
//! The bus is a thin wrapper over [`tokio::sync::broadcast`] so any number
//! of subscribers (terminal renderer, log writer, scripting engine, ...)
//! can tap in without coupling to each other.
//!
//! ## Subscription timing
//!
//! Broadcast channels do **not** replay history for late subscribers — only
//! events sent *after* a subscription are observable. Subscribe via
//! [`EventBus::subscribe`] before any code that may publish events of
//! interest, typically before calling [`Session::run`](crate::Session::run).

use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::broadcast;

use crate::command::Command;
use crate::config::SerialConfig;
use crate::error::Error;

/// Default channel capacity. Large enough to absorb burst traffic from
/// 3 Mbaud ports while keeping memory bounded; lagging subscribers see
/// [`broadcast::error::RecvError::Lagged`] and can resync.
pub const DEFAULT_BUS_CAPACITY: usize = 1024;

/// One unit of work that flowed through (or originated inside) a session.
///
/// `#[non_exhaustive]` so future variants (`UserInput`, `Command`, ...)
/// added in later issues do not break downstream code that matches.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    /// Bytes just read from the serial device.
    RxBytes(Bytes),
    /// Bytes pending transmission to the serial device. Publishing this
    /// asks the writer task to send them.
    TxBytes(Bytes),
    /// A runtime command produced by the keyboard state machine
    /// (Issue #6); subscribed by the command-handler dispatcher in
    /// Issue #7.
    Command(Command),
    /// The session opened the device and is ready to do I/O.
    DeviceConnected,
    /// The session lost the device (EOF, write failure, hot-unplug).
    DeviceDisconnected {
        /// Human-readable reason intended for logs and the status bar.
        reason: String,
    },
    /// The serial configuration changed at runtime (e.g. `^T b 9600`).
    ConfigChanged(SerialConfig),
    /// Human-readable status text emitted by the session itself
    /// (Help banner, `ShowConfig`, line-toggle acknowledgements, ...).
    /// The terminal renderer renders these with a `*** rtcom: ` prefix
    /// to keep them distinct from serial data; log writers
    /// (Issue #10) must drop them so they do not pollute capture
    /// files.
    SystemMessage(String),
    /// A non-fatal error worth surfacing to subscribers. Wrapped in `Arc`
    /// so the broadcast channel can clone it cheaply across receivers.
    Error(Arc<Error>),
    /// The TUI menu opened. Informational signal so log writers / scripts
    /// can react (e.g., pause disk flushing while the UI is interactive).
    MenuOpened,
    /// The TUI menu closed.
    MenuClosed,
    /// A profile was successfully written to disk.
    ProfileSaved {
        /// Destination path on disk.
        path: PathBuf,
    },
    /// A profile read or write failed. The session continues with the
    /// last-known-good configuration; subscribers surface this to the user
    /// (e.g. as a toast) but must not treat it as fatal.
    ProfileLoadFailed {
        /// Path that failed to load or save.
        path: PathBuf,
        /// Source error, shareable across the broadcast fan-out.
        error: Arc<Error>,
    },
    /// DTR / RTS output-line state changed. Published by the session
    /// after a successful
    /// [`ToggleDtr`](crate::command::Command::ToggleDtr) /
    /// [`ToggleRts`](crate::command::Command::ToggleRts) /
    /// [`SetDtrAbs`](crate::command::Command::SetDtrAbs) /
    /// [`SetRtsAbs`](crate::command::Command::SetRtsAbs) dispatch so
    /// subscribers (notably the TUI) can refresh their cached
    /// [`ModemLineSnapshot`](crate::config::ModemLineSnapshot) without
    /// re-reading the device.
    ModemLinesChanged {
        /// Current DTR state after the change.
        dtr: bool,
        /// Current RTS state after the change.
        rts: bool,
    },
}

/// Multi-producer, multi-consumer event hub.
///
/// `EventBus` is `Clone` because it is meant to be handed to as many tasks
/// as need to publish or subscribe; clones share the same underlying
/// channel.
#[derive(Clone, Debug)]
pub struct EventBus {
    inner: broadcast::Sender<Event>,
}

impl EventBus {
    /// Creates a new bus with the given channel capacity.
    ///
    /// A capacity of zero is silently raised to one so the underlying
    /// broadcast channel does not panic.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { inner: tx }
    }

    /// Publishes an event to all current subscribers.
    ///
    /// Returns the number of subscribers that received the event, or 0 if
    /// none were attached. Unlike [`broadcast::Sender::send`], a missing
    /// subscriber is *not* treated as an error: events are best-effort and
    /// callers should not block their own work because nobody is listening.
    pub fn publish(&self, event: Event) -> usize {
        self.inner.send(event).unwrap_or(0)
    }

    /// Returns a fresh subscription that yields every event published from
    /// this point on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.subscribe()
    }

    /// Returns the current number of active subscribers.
    #[must_use]
    pub fn receiver_count(&self) -> usize {
        self.inner.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(DEFAULT_BUS_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_round_trips_to_subscribers() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        let delivered = bus.publish(Event::DeviceConnected);
        assert_eq!(delivered, 1);
        assert!(matches!(rx.recv().await.unwrap(), Event::DeviceConnected));
    }

    #[tokio::test]
    async fn publish_with_no_subscribers_returns_zero() {
        let bus = EventBus::new(8);
        assert_eq!(bus.publish(Event::DeviceConnected), 0);
    }

    #[tokio::test]
    async fn system_message_round_trips() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        bus.publish(Event::SystemMessage("hello".into()));
        match rx.recv().await.unwrap() {
            Event::SystemMessage(text) => assert_eq!(text, "hello"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn command_event_round_trips() {
        use crate::Command;
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        bus.publish(Event::Command(Command::Quit));
        match rx.recv().await.unwrap() {
            Event::Command(Command::Quit) => {}
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn each_subscriber_sees_each_event() {
        let bus = EventBus::new(8);
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.publish(Event::DeviceConnected);
        assert!(matches!(a.recv().await.unwrap(), Event::DeviceConnected));
        assert!(matches!(b.recv().await.unwrap(), Event::DeviceConnected));
    }

    #[test]
    fn zero_capacity_is_promoted_to_one() {
        // Mostly a smoke check: broadcast::channel(0) panics; we must not.
        let _bus = EventBus::new(0);
    }

    #[test]
    fn event_menu_opened_closed_are_clone() {
        const fn assert_clone<T: Clone>() {}
        assert_clone::<Event>();
        assert!(matches!(Event::MenuOpened, Event::MenuOpened));
        assert!(matches!(Event::MenuClosed, Event::MenuClosed));
    }

    #[test]
    fn event_profile_saved_has_path() {
        let ev = Event::ProfileSaved {
            path: std::path::PathBuf::from("/tmp/x.toml"),
        };
        match ev {
            Event::ProfileSaved { path } => {
                assert_eq!(path, std::path::PathBuf::from("/tmp/x.toml"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn event_modem_lines_changed_carries_both_booleans() {
        let ev = Event::ModemLinesChanged {
            dtr: true,
            rts: false,
        };
        match ev {
            Event::ModemLinesChanged { dtr, rts } => {
                assert!(dtr);
                assert!(!rts);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn event_profile_load_failed_has_path_and_error() {
        use std::sync::Arc;
        let err = crate::error::Error::InvalidConfig("boom".into());
        let ev = Event::ProfileLoadFailed {
            path: std::path::PathBuf::from("/tmp/bad.toml"),
            error: Arc::new(err),
        };
        match ev {
            Event::ProfileLoadFailed { path, error } => {
                assert_eq!(path, std::path::PathBuf::from("/tmp/bad.toml"));
                assert!(error.to_string().contains("boom"));
            }
            _ => panic!("wrong variant"),
        }
    }
}
