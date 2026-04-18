//! Session orchestrator: bridges a [`SerialDevice`] with the
//! [`EventBus`].
//!
//! [`SerialDevice`]: crate::SerialDevice
//! [`EventBus`]: crate::EventBus
//!
//! At v0.1 a [`Session`] runs a single task that multiplexes the serial
//! device, the cancellation token, and the bus subscription via
//! `tokio::select!`:
//!
//! - bytes arriving from the device → [`Event::RxBytes`];
//! - [`Event::TxBytes`] published on the bus → bytes written to the device;
//! - [`Event::Command`] published on the bus → handler dispatch (Issue #7);
//! - cancellation token tripped or fatal I/O error → publish
//!   [`Event::DeviceDisconnected`] (when applicable) and exit cleanly.
//!
//! The single-task model gives the dispatch handlers exclusive `&mut`
//! access to the device, which is required for the synchronous control
//! operations (`set_baud_rate`, `set_dtr`, `send_break`, ...). The
//! tradeoff is that a long write momentarily delays reads — acceptable
//! for an interactive serial terminal where writes are short and rare.
//!
//! Later issues plug in mappers (Issue #8), logging, scripting, and so
//! on as additional bus subscribers.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::command::Command;
use crate::config::{Parity, SerialConfig, StopBits};
use crate::device::SerialDevice;
use crate::error::Result;
use crate::event::{Event, EventBus};
use crate::mapper::{LineEndingMapper, Mapper};

const fn parity_letter(p: Parity) -> char {
    match p {
        Parity::None => 'N',
        Parity::Even => 'E',
        Parity::Odd => 'O',
        Parity::Mark => 'M',
        Parity::Space => 'S',
    }
}

const fn stop_bits_number(s: StopBits) -> u8 {
    match s {
        StopBits::One => 1,
        StopBits::Two => 2,
    }
}

/// Read buffer size. 4 KiB matches the typical USB-serial driver burst
/// granularity; larger buffers waste memory, smaller ones fragment events.
const READ_BUFFER_BYTES: usize = 4096;

/// Duration of the line break asserted by the `SendBreak` command.
const SEND_BREAK_DURATION: Duration = Duration::from_millis(250);

/// Static cheatsheet text for `Command::Help`.
const HELP_TEXT: &str = "commands: ?/h help, q/x quit, c show config, t toggle DTR, \
                         g toggle RTS, b<rate><Enter> set baud, \\ send break";

/// Owns a serial device and a bus, and runs the I/O + command loop.
///
/// `Session` is generic over the device type so tests can substitute a
/// PTY pair (`SerialPortDevice::pair`) or, in the future, a fully mocked
/// backend without dynamic dispatch overhead.
pub struct Session<D: SerialDevice + 'static> {
    device: D,
    bus: EventBus,
    cancel: CancellationToken,
    /// Outbound mapper applied to `Event::TxBytes` payloads before they
    /// hit the device. Defaults to a no-op `LineEndingMapper::default()`.
    omap: Box<dyn Mapper>,
    /// Inbound mapper applied to bytes read from the device before they
    /// are republished as `Event::RxBytes`. Defaults to no-op.
    imap: Box<dyn Mapper>,
    /// Tracked DTR state. Initialised to `true` because `SerialDevice`
    /// gives no way to query the line, and most backends open with DTR
    /// asserted; the first toggle therefore deasserts.
    dtr_asserted: bool,
    /// Tracked RTS state. See `dtr_asserted` for the rationale.
    rts_asserted: bool,
}

impl<D: SerialDevice + 'static> Session<D> {
    /// Builds a session with a fresh bus and cancellation token,
    /// no-op mappers on both directions.
    #[must_use]
    pub fn new(device: D) -> Self {
        Self {
            device,
            bus: EventBus::default(),
            cancel: CancellationToken::new(),
            omap: Box::new(LineEndingMapper::default()),
            imap: Box::new(LineEndingMapper::default()),
            dtr_asserted: true,
            rts_asserted: true,
        }
    }

    /// Builds a session attached to a caller-supplied bus. Useful when
    /// several subsystems already share a bus and the session should join
    /// the existing fan-out instead of starting its own.
    #[must_use]
    pub fn with_bus(device: D, bus: EventBus) -> Self {
        Self {
            device,
            bus,
            cancel: CancellationToken::new(),
            omap: Box::new(LineEndingMapper::default()),
            imap: Box::new(LineEndingMapper::default()),
            dtr_asserted: true,
            rts_asserted: true,
        }
    }

    /// Replaces the outbound mapper applied to `Event::TxBytes`
    /// payloads before they reach the device.
    #[must_use]
    pub fn with_omap<M: Mapper + 'static>(mut self, mapper: M) -> Self {
        self.omap = Box::new(mapper);
        self
    }

    /// Replaces the inbound mapper applied to bytes read from the
    /// device before they are republished as `Event::RxBytes`.
    #[must_use]
    pub fn with_imap<M: Mapper + 'static>(mut self, mapper: M) -> Self {
        self.imap = Box::new(mapper);
        self
    }

    /// Tells the session what the DTR line's actual state is on the
    /// device. Use this when the caller has already issued a
    /// `set_dtr` (e.g. main applying `--lower-dtr` right after
    /// opening the port) so the cached state stays honest and the
    /// first `Command::ToggleDtr` produces the right transition.
    ///
    /// Defaults to `true` (asserted) — the typical OS state at open.
    #[must_use]
    pub const fn with_initial_dtr(mut self, asserted: bool) -> Self {
        self.dtr_asserted = asserted;
        self
    }

    /// Tells the session what the RTS line's actual state is. See
    /// [`with_initial_dtr`](Self::with_initial_dtr) for the rationale.
    #[must_use]
    pub const fn with_initial_rts(mut self, asserted: bool) -> Self {
        self.rts_asserted = asserted;
        self
    }

    /// Returns a reference to the bus. Clone it before calling
    /// [`Session::run`] (which consumes `self`) if you need to publish or
    /// subscribe from outside the session.
    #[must_use]
    pub const fn bus(&self) -> &EventBus {
        &self.bus
    }

    /// Returns a clone of the cancellation token.
    ///
    /// Triggering [`CancellationToken::cancel`] on any clone causes
    /// [`Session::run`] to wind down and return.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Drives the session to completion.
    ///
    /// Subscribes to the bus, publishes [`Event::DeviceConnected`], then
    /// loops until the cancellation token trips or a fatal I/O error
    /// terminates the device.
    ///
    /// # Errors
    ///
    /// Currently always returns `Ok(())`; the variant is reserved for
    /// startup failures introduced by later issues (e.g. mapper
    /// initialisation).
    pub async fn run(mut self) -> crate::Result<()> {
        // Subscribe BEFORE publishing so the loop sees nothing it sent
        // itself, but external pre-existing subscribers still get
        // DeviceConnected.
        let mut subscriber = self.bus.subscribe();
        self.bus.publish(Event::DeviceConnected);

        let mut read_buf = vec![0_u8; READ_BUFFER_BYTES];
        loop {
            tokio::select! {
                biased;
                () = self.cancel.cancelled() => break,

                res = self.device.read(&mut read_buf) => match res {
                    Ok(0) => {
                        self.bus.publish(Event::DeviceDisconnected {
                            reason: "EOF on serial read".into(),
                        });
                        break;
                    }
                    Ok(n) => {
                        let mapped = self.imap.map(&read_buf[..n]);
                        self.bus.publish(Event::RxBytes(mapped));
                    }
                    Err(err) => {
                        self.bus.publish(Event::DeviceDisconnected {
                            reason: format!("serial read failed: {err}"),
                        });
                        break;
                    }
                },

                msg = subscriber.recv() => match msg {
                    Ok(Event::TxBytes(bytes)) => {
                        let mapped = self.omap.map(&bytes);
                        if let Err(err) = self.device.write_all(&mapped).await {
                            self.bus.publish(Event::DeviceDisconnected {
                                reason: format!("serial write failed: {err}"),
                            });
                            break;
                        }
                    }
                    Ok(Event::Command(cmd)) => self.dispatch_command(cmd).await,
                    // Lagged: we missed some events but can resume.
                    // Other event variants are not the loop's concern.
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    // Closed: no senders left, nothing more will arrive.
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
        Ok(())
    }

    /// Apply a [`Command`] to the device and bus.
    ///
    /// Commands that mutate the device run synchronously here; success
    /// emits [`Event::ConfigChanged`] (`ApplyConfig` / `SetBaud`) or
    /// [`Event::ModemLinesChanged`] (line toggles / absolute sets),
    /// failure emits [`Event::Error`]. The `async` signature exists so
    /// the dispatcher can await [`Session::apply_config`] without
    /// forking a task; the other arms are synchronous and perform no
    /// awaits.
    pub(crate) async fn dispatch_command(&mut self, cmd: Command) {
        match cmd {
            Command::Quit => self.cancel.cancel(),
            Command::Help => {
                self.bus.publish(Event::SystemMessage(HELP_TEXT.into()));
            }
            Command::ShowConfig => {
                let cfg = self.device.config();
                self.bus.publish(Event::SystemMessage(format!(
                    "config: {} {}{}{} flow={:?}",
                    cfg.baud_rate,
                    cfg.data_bits.bits(),
                    parity_letter(cfg.parity),
                    stop_bits_number(cfg.stop_bits),
                    cfg.flow_control,
                )));
            }
            Command::SetBaud(rate) => match self.device.set_baud_rate(rate) {
                Ok(()) => {
                    self.bus
                        .publish(Event::ConfigChanged(*self.device.config()));
                }
                Err(err) => {
                    self.bus.publish(Event::Error(Arc::new(err)));
                }
            },
            Command::ApplyConfig(cfg) => {
                if let Err(err) = self.apply_config(cfg).await {
                    self.bus.publish(Event::Error(Arc::new(err)));
                }
                // Success path: `apply_config` already published
                // `ConfigChanged`.
            }
            Command::ToggleDtr => {
                let new_state = !self.dtr_asserted;
                self.apply_dtr(new_state);
            }
            Command::ToggleRts => {
                let new_state = !self.rts_asserted;
                self.apply_rts(new_state);
            }
            Command::SetDtrAbs(state) => self.apply_dtr(state),
            Command::SetRtsAbs(state) => self.apply_rts(state),
            Command::SendBreak => match self.device.send_break(SEND_BREAK_DURATION) {
                Ok(()) => {
                    self.bus.publish(Event::SystemMessage(format!(
                        "sent {} ms break",
                        SEND_BREAK_DURATION.as_millis()
                    )));
                }
                Err(err) => {
                    self.bus.publish(Event::Error(Arc::new(err)));
                }
            },
            Command::OpenMenu => {
                // T4 only wires the parser → event. The actual TUI
                // subscriber lands in a later task; for now just
                // broadcast the signal so late-bound listeners can
                // observe it.
                self.bus.publish(Event::MenuOpened);
            }
        }
    }

    /// Drive the DTR line to `new_state`, publishing a `SystemMessage`
    /// and a [`Event::ModemLinesChanged`] on success, or
    /// [`Event::Error`] on failure. Shared by `ToggleDtr` and
    /// `SetDtrAbs` so both paths surface identical observable events.
    fn apply_dtr(&mut self, new_state: bool) {
        match self.device.set_dtr(new_state) {
            Ok(()) => {
                self.dtr_asserted = new_state;
                self.bus.publish(Event::SystemMessage(format!(
                    "DTR: {}",
                    if new_state { "asserted" } else { "deasserted" }
                )));
                self.bus.publish(Event::ModemLinesChanged {
                    dtr: self.dtr_asserted,
                    rts: self.rts_asserted,
                });
            }
            Err(err) => {
                self.bus.publish(Event::Error(Arc::new(err)));
            }
        }
    }

    /// RTS counterpart to [`Self::apply_dtr`].
    fn apply_rts(&mut self, new_state: bool) {
        match self.device.set_rts(new_state) {
            Ok(()) => {
                self.rts_asserted = new_state;
                self.bus.publish(Event::SystemMessage(format!(
                    "RTS: {}",
                    if new_state { "asserted" } else { "deasserted" }
                )));
                self.bus.publish(Event::ModemLinesChanged {
                    dtr: self.dtr_asserted,
                    rts: self.rts_asserted,
                });
            }
            Err(err) => {
                self.bus.publish(Event::Error(Arc::new(err)));
            }
        }
    }

    /// Apply a new [`SerialConfig`] to the device atomically.
    ///
    /// Applies `baud_rate → data_bits → stop_bits → parity → flow_control`
    /// in that fixed order. On the first failing step, best-effort-rolls
    /// back the previously-applied steps to the configuration that was
    /// live at entry, returns the [`Error`](crate::Error) from the failing
    /// step, and does not publish [`Event::ConfigChanged`]. On full
    /// success, publishes [`Event::ConfigChanged`] with the device's
    /// post-apply configuration and returns `Ok(())`.
    ///
    /// Fields whose new value equals the current value still go through
    /// the setter call — the backend is free to short-circuit, and keeping
    /// the apply sequence uniform avoids branchy rollback state.
    ///
    /// This method is `async` for forward compatibility with backends
    /// whose setters may need to await (e.g. remote devices); the current
    /// `serialport` backend is synchronous so the body performs no
    /// awaits.
    ///
    /// # Errors
    ///
    /// Returns the first setter failure encountered. Rollback failures
    /// are best-effort and silently swallowed — the device is already in
    /// an inconsistent state by that point and surfacing a secondary
    /// error would mask the original cause.
    // `async` is deliberate: the public API is async so a future backend
    // (e.g. a networked device whose setters must round-trip) can plug in
    // without a breaking signature change. The current synchronous path
    // simply performs no awaits.
    #[allow(clippy::unused_async)]
    pub async fn apply_config(&mut self, new: SerialConfig) -> Result<()> {
        let snapshot = *self.device.config();

        if let Err(e) = self.device.set_baud_rate(new.baud_rate) {
            self.rollback(&snapshot);
            return Err(e);
        }
        if let Err(e) = self.device.set_data_bits(new.data_bits) {
            self.rollback(&snapshot);
            return Err(e);
        }
        if let Err(e) = self.device.set_stop_bits(new.stop_bits) {
            self.rollback(&snapshot);
            return Err(e);
        }
        if let Err(e) = self.device.set_parity(new.parity) {
            self.rollback(&snapshot);
            return Err(e);
        }
        if let Err(e) = self.device.set_flow_control(new.flow_control) {
            self.rollback(&snapshot);
            return Err(e);
        }

        self.bus
            .publish(Event::ConfigChanged(*self.device.config()));
        Ok(())
    }

    /// Best-effort rollback to `snapshot`. Errors are intentionally
    /// ignored: the device is already inconsistent, and we prefer to
    /// surface the original failure to the caller.
    fn rollback(&mut self, snapshot: &SerialConfig) {
        let _ = self.device.set_baud_rate(snapshot.baud_rate);
        let _ = self.device.set_data_bits(snapshot.data_bits);
        let _ = self.device.set_stop_bits(snapshot.stop_bits);
        let _ = self.device.set_parity(snapshot.parity);
        let _ = self.device.set_flow_control(snapshot.flow_control);
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for [`Session::apply_config`] using an in-module
    //! [`MockDevice`]. The mock is intentionally not exposed outside
    //! this file — integration tests use [`crate::SerialPortDevice::pair`]
    //! which offers a real PTY but cannot drive setter failures.

    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::sync::broadcast::error::TryRecvError;

    use super::{Event, Result, SerialDevice, Session};
    use crate::command::Command;
    use crate::config::{DataBits, FlowControl, ModemStatus, Parity, SerialConfig, StopBits};
    use crate::error::Error;

    /// In-memory [`SerialDevice`] with programmable setter failures.
    ///
    /// Each setter can be armed to fail on its next call via the
    /// corresponding `fail_*` flag; the flag consumes itself (one-shot)
    /// so a rearmed setter fails exactly once.
    //
    // The five booleans model five independent one-shot triggers on the
    // five distinct setters; a state machine or enum would be strictly
    // more awkward for this "pick which steps blow up" harness.
    #[allow(clippy::struct_excessive_bools)]
    struct MockDevice {
        config: SerialConfig,
        fail_baud: bool,
        fail_data_bits: bool,
        fail_stop_bits: bool,
        fail_parity: bool,
        fail_flow: bool,
    }

    impl MockDevice {
        const fn new(config: SerialConfig) -> Self {
            Self {
                config,
                fail_baud: false,
                fail_data_bits: false,
                fail_stop_bits: false,
                fail_parity: false,
                fail_flow: false,
            }
        }
    }

    impl AsyncRead for MockDevice {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for MockDevice {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl SerialDevice for MockDevice {
        fn set_baud_rate(&mut self, baud: u32) -> Result<()> {
            if self.fail_baud {
                self.fail_baud = false;
                return Err(Error::InvalidConfig("mock: baud fail".into()));
            }
            self.config.baud_rate = baud;
            Ok(())
        }
        fn set_data_bits(&mut self, bits: DataBits) -> Result<()> {
            if self.fail_data_bits {
                self.fail_data_bits = false;
                return Err(Error::InvalidConfig("mock: data_bits fail".into()));
            }
            self.config.data_bits = bits;
            Ok(())
        }
        fn set_stop_bits(&mut self, bits: StopBits) -> Result<()> {
            if self.fail_stop_bits {
                self.fail_stop_bits = false;
                return Err(Error::InvalidConfig("mock: stop_bits fail".into()));
            }
            self.config.stop_bits = bits;
            Ok(())
        }
        fn set_parity(&mut self, parity: Parity) -> Result<()> {
            if self.fail_parity {
                self.fail_parity = false;
                return Err(Error::InvalidConfig("mock: parity fail".into()));
            }
            self.config.parity = parity;
            Ok(())
        }
        fn set_flow_control(&mut self, flow: FlowControl) -> Result<()> {
            if self.fail_flow {
                self.fail_flow = false;
                return Err(Error::InvalidConfig("mock: flow fail".into()));
            }
            self.config.flow_control = flow;
            Ok(())
        }
        fn set_dtr(&mut self, _level: bool) -> Result<()> {
            Ok(())
        }
        fn set_rts(&mut self, _level: bool) -> Result<()> {
            Ok(())
        }
        fn send_break(&mut self, _duration: Duration) -> Result<()> {
            Ok(())
        }
        fn modem_status(&mut self) -> Result<ModemStatus> {
            Ok(ModemStatus::default())
        }
        fn config(&self) -> &SerialConfig {
            &self.config
        }
    }

    fn new_cfg() -> SerialConfig {
        SerialConfig {
            baud_rate: 9600,
            data_bits: DataBits::Seven,
            stop_bits: StopBits::Two,
            parity: Parity::Even,
            flow_control: FlowControl::Hardware,
            ..SerialConfig::default()
        }
    }

    #[tokio::test]
    async fn apply_config_success_publishes_config_changed() {
        let device = MockDevice::new(SerialConfig::default());
        let mut session = Session::new(device);
        let mut rx = session.bus().subscribe();

        let target = new_cfg();
        session
            .apply_config(target)
            .await
            .expect("apply_config should succeed");

        // Device state reflects the new config.
        let got = session.device.config();
        assert_eq!(got.baud_rate, target.baud_rate);
        assert_eq!(got.data_bits, target.data_bits);
        assert_eq!(got.stop_bits, target.stop_bits);
        assert_eq!(got.parity, target.parity);
        assert_eq!(got.flow_control, target.flow_control);

        // Event::ConfigChanged was published with the new config.
        match rx.try_recv() {
            Ok(Event::ConfigChanged(cfg)) => {
                assert_eq!(cfg.baud_rate, target.baud_rate);
                assert_eq!(cfg.flow_control, target.flow_control);
            }
            other => panic!("expected ConfigChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn apply_config_rolls_back_on_middle_failure() {
        // Start at default, arm the flow-control setter to fail.
        let mut device = MockDevice::new(SerialConfig::default());
        device.fail_flow = true;
        let initial = *device.config();

        let mut session = Session::new(device);
        let mut rx = session.bus().subscribe();

        let target = new_cfg();
        let err = session
            .apply_config(target)
            .await
            .expect_err("apply_config must fail when flow setter errors");
        assert!(matches!(err, Error::InvalidConfig(_)));

        // Device state was rolled back to the pre-apply snapshot.
        let got = session.device.config();
        assert_eq!(got.baud_rate, initial.baud_rate);
        assert_eq!(got.data_bits, initial.data_bits);
        assert_eq!(got.stop_bits, initial.stop_bits);
        assert_eq!(got.parity, initial.parity);
        assert_eq!(got.flow_control, initial.flow_control);

        // No ConfigChanged event was published.
        match rx.try_recv() {
            Err(TryRecvError::Empty) => {}
            Ok(Event::ConfigChanged(_)) => panic!("unexpected ConfigChanged after rollback"),
            other => panic!("unexpected bus state: {other:?}"),
        }
    }

    #[tokio::test]
    async fn apply_config_command_dispatches_through_session() {
        let device = MockDevice::new(SerialConfig::default());
        let mut session = Session::new(device);
        let mut rx = session.bus().subscribe();

        let target = SerialConfig {
            baud_rate: 9600,
            ..SerialConfig::default()
        };
        session.dispatch_command(Command::ApplyConfig(target)).await;

        let ev = rx.try_recv().expect("ConfigChanged should be on the bus");
        match ev {
            Event::ConfigChanged(cfg) => assert_eq!(cfg.baud_rate, 9600),
            other => panic!("expected ConfigChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn apply_config_command_on_failure_publishes_error() {
        let mut device = MockDevice::new(SerialConfig::default());
        device.fail_baud = true;
        let mut session = Session::new(device);
        let mut rx = session.bus().subscribe();

        let target = SerialConfig {
            baud_rate: 9600,
            ..SerialConfig::default()
        };
        session.dispatch_command(Command::ApplyConfig(target)).await;

        match rx.try_recv() {
            Ok(Event::Error(_)) => {}
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_dtr_abs_publishes_modem_lines_changed() {
        let device = MockDevice::new(SerialConfig::default());
        let mut session = Session::new(device);
        let mut rx = session.bus().subscribe();

        session.dispatch_command(Command::SetDtrAbs(true)).await;

        // Expected sequence: SystemMessage, ModemLinesChanged.
        match rx.recv().await.unwrap() {
            Event::SystemMessage(_) => {}
            other => panic!("expected SystemMessage, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            Event::ModemLinesChanged { dtr, rts } => {
                assert!(dtr);
                // `rts_asserted` defaults to `true` in Session::new.
                assert!(rts);
            }
            other => panic!("expected ModemLinesChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_rts_abs_publishes_modem_lines_changed() {
        let device = MockDevice::new(SerialConfig::default());
        let mut session = Session::new(device);
        let mut rx = session.bus().subscribe();

        session.dispatch_command(Command::SetRtsAbs(false)).await;

        let _ = rx.recv().await; // SystemMessage
        match rx.recv().await.unwrap() {
            Event::ModemLinesChanged { dtr, rts } => {
                assert!(dtr);
                assert!(!rts);
            }
            other => panic!("expected ModemLinesChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn toggle_dtr_now_also_publishes_modem_lines_changed() {
        let device = MockDevice::new(SerialConfig::default());
        let mut session = Session::new(device);
        let mut rx = session.bus().subscribe();

        session.dispatch_command(Command::ToggleDtr).await;

        let _ = rx.recv().await; // SystemMessage (existing pre-T17 behaviour)
        match rx.recv().await.unwrap() {
            Event::ModemLinesChanged { dtr, rts } => {
                // Toggle from the default (true) lowers DTR.
                assert!(!dtr);
                assert!(rts);
            }
            other => panic!("expected ModemLinesChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn toggle_rts_now_also_publishes_modem_lines_changed() {
        let device = MockDevice::new(SerialConfig::default());
        let mut session = Session::new(device);
        let mut rx = session.bus().subscribe();

        session.dispatch_command(Command::ToggleRts).await;

        let _ = rx.recv().await; // SystemMessage
        match rx.recv().await.unwrap() {
            Event::ModemLinesChanged { dtr, rts } => {
                assert!(dtr);
                assert!(!rts);
            }
            other => panic!("expected ModemLinesChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn apply_config_rolls_back_on_first_step_failure() {
        // Arm baud to fail — the very first step.
        let mut device = MockDevice::new(SerialConfig::default());
        device.fail_baud = true;
        let initial = *device.config();

        let mut session = Session::new(device);
        let mut rx = session.bus().subscribe();

        let target = new_cfg();
        let err = session
            .apply_config(target)
            .await
            .expect_err("apply_config must fail when baud setter errors");
        assert!(matches!(err, Error::InvalidConfig(_)));

        // Device state is unchanged (rollback is a no-op since nothing
        // succeeded before the failing step, but we still verify).
        let got = session.device.config();
        assert_eq!(got, &initial);

        // No ConfigChanged event was published.
        match rx.try_recv() {
            Err(TryRecvError::Empty) => {}
            Ok(Event::ConfigChanged(_)) => panic!("unexpected ConfigChanged after rollback"),
            other => panic!("unexpected bus state: {other:?}"),
        }
    }
}
