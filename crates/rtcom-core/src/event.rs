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
    /// A non-fatal error worth surfacing to subscribers. Wrapped in `Arc`
    /// so the broadcast channel can clone it cheaply across receivers.
    Error(Arc<Error>),
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
}
