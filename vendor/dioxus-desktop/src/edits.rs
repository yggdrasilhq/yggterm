//! The internal edit queue facilitating native <-> webview communication.
//!
//! Originally, we used long-polling on the wry custom protocol to send edits to the webview.
//! Due to bugs in wry on android, we switched to a websocket connection that the webview connects to.
//! We use the sledgehammer crate to build batches of edits and send them through the websocket to
//! the webview.
//!
//! Using a websocket lets us send binary data to the webview quite efficiently and does encounter
//! many of the issues with regular request/response protocols. Note that the websocket max frame
//! size is quite large (9.22 exabytes), so we can have very large batches without issue.
//!
//! Using websockets does mean we need to handle security and content security policies ourselves.
//! The code here generates a random key that the webview must use to connect to the websocket.
//! We use the initialization script API to setup the websocket connection without leaking the key
//! to the webview itself in case there's untrusted content in the webview.
//!
//! Some operating systems (like iOS) will kill the websocket connection when the device goes to sleep.
//! If this happens, we will automatically switch to a new port and notify the webview of the new location
//! and key. The webview will then reconnect to the new port and continue receiving edits.

use dioxus_interpreter_js::MutationState;
use futures_channel::oneshot;
use futures_util::FutureExt;
use rand::{RngCore, SeedableRng};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::net::{TcpListener, TcpStream};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::AtomicU32;
use std::sync::Mutex;
use std::{
    net::IpAddr,
    sync::{Arc, RwLock},
};
use tokio::sync::Notify;

/// How many edit batches the webview has reported it could not fully apply.
///
/// Non-zero means the DOM and the `VirtualDom`'s model of it have diverged and
/// cannot converge on their own — every count is a subtree that stopped tracking
/// its state. Process-wide and monotonic, because the damage is too: nothing
/// clears it short of a restart. Read it with [`edit_faults`].
static EDIT_FAULTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Edit batches the webview failed to apply since launch. See [`EDIT_FAULTS`].
pub fn edit_faults() -> u64 {
    EDIT_FAULTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Times the flush gate timed out waiting for the webview to acknowledge an
/// edit batch (see the deadline in `poll_edits_flushed`). Every hit is a
/// window in which the whole VirtualDom — renders, effects, spawned futures —
/// sat frozen for the full timeout while the event loop looked healthy. The
/// gate releases and the app recovers, which is exactly why the class hid: it
/// logged one line and left no queryable trace. Monotonic; a host process
/// polls it and files the incident (the emitter cannot — this crate has no
/// trace plane).
static EDIT_FLUSH_TIMEOUTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Flush-gate timeouts since launch. See [`EDIT_FLUSH_TIMEOUTS`].
pub fn edit_flush_timeouts() -> u64 {
    EDIT_FLUSH_TIMEOUTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Acknowledgements that arrived AFTER the flush gate had given up on them.
///
/// ⭐ THIS IS THE FIELD THAT SAYS WHETHER THE UI IS ACTUALLY STALE. A gate
/// timeout on its own does not: the batch is delivered over the websocket and
/// the interpreter applies it before acking, and the acknowledgement itself has
/// a `setTimeout` backstop on the JS side for exactly the occluded-window case.
/// So a timeout followed by a late ack means the webview was merely SLOW — the
/// DOM caught up and nothing is stale. A streak of timeouts with no late ack
/// means the ack plane is dead, which is a different fault with a different
/// remedy. Counting them apart is what stops "the UI may be one frame stale"
/// from being said about a webview that was one second behind.
static EDIT_ACKS_LATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Acknowledgements that arrived after the gate gave up. See [`EDIT_ACKS_LATE`].
pub fn edit_acks_late() -> u64 {
    EDIT_ACKS_LATE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Batches sent while the flush gate was BYPASSED — i.e. the VirtualDom was
/// deliberately not frozen, because the webview already owes us
/// [`EDIT_FLUSH_GATE_BYPASS_AFTER`] acknowledgements and freezing it for every
/// further batch is the input-death the owner feels, not a fix for it.
static EDIT_GATE_BYPASSES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Batches that skipped the flush gate. See [`EDIT_GATE_BYPASSES`].
pub fn edit_flush_gate_bypasses() -> u64 {
    EDIT_GATE_BYPASSES.load(std::sync::atomic::Ordering::Relaxed)
}

/// Times the ack plane was judged DEAD and a webview resync was asked for.
static EDIT_RESYNC_REQUESTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Webview resyncs requested by the recovery ladder. See [`EDIT_RESYNC_REQUESTS`].
pub fn edit_webview_resync_requests() -> u64 {
    EDIT_RESYNC_REQUESTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Unacknowledged batches after which the gate stops freezing the VirtualDom.
///
/// Two is a slow frame; three in a row is a webview that is not keeping its end
/// of the bargain, and the gate is only an ORDERING nicety — worth a wait, never
/// worth a third consecutive freeze.
pub const EDIT_FLUSH_GATE_BYPASS_AFTER: u32 = 3;

/// Unacknowledged batches after which the ack plane is judged dead.
pub const EDIT_ACK_DEAD_BATCHES: u32 = 12;

/// How long the streak must ALSO have lasted before the ack plane is judged
/// dead. ⛔ The batch count alone is not enough: once the gate is bypassed the
/// VirtualDom runs freely and can produce a dozen batches in a second, so a
/// count-only rule would call a webview dead a second after it went quiet.
pub const EDIT_ACK_DEAD_MS: u64 = 20_000;

/// What to do about a batch the webview has not acknowledged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditFlushRecovery {
    /// Wait for the acknowledgement, as designed.
    Gate,
    /// Stop waiting. The webview owes us acks; freezing the whole VirtualDom
    /// for each further batch turns one slow webview into a dead application.
    BypassGate,
    /// The ack plane has not answered at all for long enough that the page is
    /// no longer a live surface. Reload it.
    ReloadWebview,
}

/// The recovery ladder, kept pure so it can be tested without a webview.
///
/// `unacked_batches` counts batches sent since the last acknowledgement OF ANY
/// KIND — timely or late. A single late ack resets it, which is the whole
/// point: a webview that answers, however slowly, is never reloaded.
pub(crate) fn edit_flush_recovery(
    unacked_batches: u32,
    ms_since_last_ack: u64,
) -> EditFlushRecovery {
    if unacked_batches >= EDIT_ACK_DEAD_BATCHES && ms_since_last_ack >= EDIT_ACK_DEAD_MS {
        return EditFlushRecovery::ReloadWebview;
    }
    if unacked_batches >= EDIT_FLUSH_GATE_BYPASS_AFTER {
        return EditFlushRecovery::BypassGate;
    }
    EditFlushRecovery::Gate
}

/// This handles communication between the requests that the webview makes and the interpreter.
#[derive(Clone)]
pub(crate) struct WryQueue {
    inner: Rc<RefCell<WryQueueInner>>,
}

impl WryQueue {
    pub(crate) fn with_mutation_state_mut<O: 'static>(
        &self,
        callback: impl FnOnce(&mut MutationState) -> O,
    ) -> O {
        let mut inner = self.inner.borrow_mut();
        callback(&mut inner.mutation_state)
    }

    /// Send a list of mutations to the webview
    pub(crate) fn send_edits(&self) {
        let mut myself = self.inner.borrow_mut();
        let webview_id = myself.location.webview_id;
        let serialized_edits = myself.mutation_state.export_memory();
        let receiver = myself.websocket.send_edits(webview_id, serialized_edits);
        myself.edits_in_progress = Some(receiver);
        myself.edits_deadline = Some(Box::pin(tokio::time::sleep(EDITS_FLUSH_TIMEOUT)));
        myself.unacked_batches = myself.unacked_batches.saturating_add(1);
    }

    /// Whether the recovery ladder has asked for a webview reload, clearing the
    /// request. Called by the poll loop, which is the only place holding the
    /// webview handle — this module can diagnose the dead ack plane but cannot
    /// reach the thing that fixes it.
    pub(crate) fn take_resync_request(&self) -> bool {
        let mut self_mut = self.inner.borrow_mut();
        std::mem::take(&mut self_mut.resync_requested)
    }

    /// Wait until all pending edits have been rendered in the webview
    pub(crate) fn poll_edits_flushed(
        &self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let mut self_mut = self.inner.borrow_mut();
        // Late acknowledgements first, and unconditionally — a batch the gate
        // gave up on can still be answered, and that answer is the evidence
        // that the DOM caught up and the webview is alive. Polling here also
        // registers our waker with those receivers.
        loop {
            let Some(front) = self_mut.abandoned_edits.front_mut() else {
                break;
            };
            if front.poll_unpin(cx).is_ready() {
                self_mut.abandoned_edits.pop_front();
                EDIT_ACKS_LATE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self_mut.note_edits_acknowledged();
            } else {
                break;
            }
        }
        if self_mut.edits_in_progress.is_none() {
            return std::task::Poll::Ready(());
        }

        let flushed = self_mut
            .edits_in_progress
            .as_mut()
            .expect("checked above")
            .poll_unpin(cx)
            .is_ready();
        if flushed {
            self_mut.edits_in_progress = None;
            self_mut.edits_deadline = None;
            self_mut.note_edits_acknowledged();
            return std::task::Poll::Ready(());
        }

        // The webview may never acknowledge a batch. Two ways it happens in
        // practice, both observed: applying the edits throws inside
        // `run_from_bytes`, or the `requestAnimationFrame` that carries the ack
        // never fires because the window is occluded and the compositor has
        // stopped delivering frame callbacks.
        //
        // This gate is a best-effort ORDERING nicety — it exists so effects do
        // not run against a DOM that has not caught up yet. It is not a
        // correctness invariant, and it must never be able to outlive the
        // webview's answer: `poll_vdom` returns early while it is held, so a
        // permanently-held gate starves EVERY task on the VirtualDom — renders,
        // effects and spawned futures alike — while the event loop itself stays
        // healthy and idle. That presents as a completely frozen app that no
        // OS-level instrument can distinguish from a well-behaved idle one.
        //
        // So: wait, but not forever. Polling the deadline here also registers
        // our waker with the timer, which is what guarantees we are polled again
        // to observe the timeout at all.
        //
        // ⭐ AND WAIT LESS EACH TIME IT HAPPENS. Waiting the full window for
        // every batch of a webview that is already several acknowledgements
        // behind multiplies one slow surface into a dead application: 13 gate
        // timeouts in one GUI's last 17 minutes is 26 seconds during which
        // renders, effects and spawned futures were all stopped, and the owner
        // fought "input blocked" for that whole stretch. So the ladder below
        // gates, then stops gating, then — only if nothing is ever
        // acknowledged, late or otherwise — declares the page dead.
        let ms_since_last_ack = self_mut.last_ack_at.elapsed().as_millis() as u64;
        let unacked = self_mut.unacked_batches;
        match edit_flush_recovery(unacked, ms_since_last_ack) {
            EditFlushRecovery::Gate => {}
            EditFlushRecovery::BypassGate => {
                EDIT_GATE_BYPASSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self_mut.abandon_pending_edits();
                return std::task::Poll::Ready(());
            }
            EditFlushRecovery::ReloadWebview => {
                let webview_id = self_mut.location.webview_id;
                EDIT_RESYNC_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self_mut.resync_requested = true;
                tracing::error!(
                    webview_id,
                    unacked_batches = unacked,
                    ms_since_last_ack,
                    "webview has not acknowledged ANY edit batch for the whole streak; \
                     asking for a reload rather than rendering into a surface that is \
                     no longer listening"
                );
                self_mut.abandon_pending_edits();
                // ⛔ AND START THE MEASUREMENT AGAIN, or the very next batch
                // sees the same spent streak and asks for another reload —
                // a reload loop, which is strictly worse than the fault it
                // is recovering from. The reload gets one full streak to
                // prove it worked before the ladder may fire again.
                self_mut.abandoned_edits.clear();
                self_mut.note_edits_acknowledged();
                return std::task::Poll::Ready(());
            }
        }

        let timed_out = match self_mut.edits_deadline.as_mut() {
            Some(deadline) => deadline.as_mut().poll(cx).is_ready(),
            None => false,
        };
        if timed_out {
            let webview_id = self_mut.location.webview_id;
            EDIT_FLUSH_TIMEOUTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracing::error!(
                webview_id,
                timeout_ms = EDITS_FLUSH_TIMEOUT.as_millis() as u64,
                unacked_batches = self_mut.unacked_batches,
                "webview never acknowledged an edit batch; releasing the flush gate \
                 so the VirtualDom keeps running (the UI may be one frame stale \
                 until the acknowledgement arrives late)"
            );
            self_mut.abandon_pending_edits();
            return std::task::Poll::Ready(());
        }

        std::task::Poll::Pending
    }

    /// Check if there is a new location for the websocket edits server.
    pub(crate) fn poll_new_edits_location(
        &self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let mut self_mut = self.inner.borrow_mut();
        let poll = self_mut
            .server_location_changed_future
            .as_mut()
            .poll_unpin(cx);
        if poll.is_ready() {
            // If the future is ready, we need to reset it to wait for the next change
            self_mut.server_location_changed_future =
                owned_notify_future(self_mut.server_location_changed.clone());
        }
        poll
    }

    /// Get the websocket path that the webview should connect to in order to receive edits
    pub(crate) fn edits_path(&self) -> String {
        let WebviewWebsocketLocation {
            webview_id, server, ..
        } = &self.inner.borrow().location;
        let server = server.lock().unwrap();
        let port = server.port;
        let key = &server.client_key;
        let key_hex = encode_key_string(key);
        format!("ws://127.0.0.1:{port}/{webview_id}/{key_hex}")
    }

    /// Get the key the client should expect from the server when connecting to the websocket.
    pub(crate) fn required_server_key(&self) -> String {
        let server = &self.inner.borrow().location.server;
        let server = server.lock().unwrap();
        encode_key_string(&server.server_key)
    }
}

/// How long to wait for the webview to acknowledge an edit batch before giving
/// up on it and letting the VirtualDom run again. Generous enough that a slow
/// frame never trips it, short enough that a lost acknowledgement is a blip
/// rather than a hang.
const EDITS_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

pub(crate) struct WryQueueInner {
    location: WebviewWebsocketLocation,
    websocket: EditWebsocket,
    // If this webview is currently waiting for an edit to be flushed. We don't run the virtual dom while this is true to avoid running effects before the dom has been updated
    edits_in_progress: Option<oneshot::Receiver<()>>,
    // Deadline for the above. See `poll_edits_flushed` for why a missing
    // acknowledgement must not be able to wedge the VirtualDom forever.
    edits_deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    // Receivers for batches the gate gave up on, oldest first. ⛔ NOT dropped:
    // the websocket thread serialises one batch at a time and answers them in
    // order, so the front of this queue is the next acknowledgement the webview
    // will produce — and observing it is the only way to tell a SLOW webview
    // (late acks arriving, DOM fine) from a DEAD one (nothing, ever). Dropping
    // the receiver instead, which is what the code did before, threw away that
    // distinction and left every timeout looking equally fatal.
    abandoned_edits: VecDeque<oneshot::Receiver<()>>,
    // Batches sent since the last acknowledgement of any kind.
    unacked_batches: u32,
    // When the webview last acknowledged anything. Starts at construction so a
    // webview that never acks at all still ages into the dead verdict.
    last_ack_at: std::time::Instant,
    // Set when the ladder asks for a reload; taken by the poll loop, which is
    // the only place that holds the webview handle.
    resync_requested: bool,
    // The socket may be killed by the OS while running. If it does, this channel will receive the new server location
    server_location_changed: Arc<Notify>,
    server_location_changed_future: Pin<Box<dyn Future<Output = ()>>>,
    mutation_state: MutationState,
}

impl WryQueueInner {
    /// The webview answered. Everything the ladder measures is measured from
    /// here — a single acknowledgement, however late, means the surface is
    /// alive and the streak starts again from nothing.
    fn note_edits_acknowledged(&mut self) {
        self.unacked_batches = 0;
        self.last_ack_at = std::time::Instant::now();
    }

    /// Release the gate on the in-flight batch while KEEPING its receiver, so a
    /// late acknowledgement is still observed rather than thrown away.
    fn abandon_pending_edits(&mut self) {
        if let Some(receiver) = self.edits_in_progress.take() {
            // Bounded: the websocket answers in order, so anything older than
            // the last few is of no diagnostic value, and an unbounded queue on
            // a webview that never answers is a leak.
            if self.abandoned_edits.len() >= ABANDONED_EDIT_ACK_WATCH {
                self.abandoned_edits.pop_front();
            }
            self.abandoned_edits.push_back(receiver);
        }
        self.edits_deadline = None;
    }
}

/// How many un-acknowledged batches to keep watching for a late answer.
const ABANDONED_EDIT_ACK_WATCH: usize = 8;

/// The location of a webview websocket connection. This is used to identify the webview and the port it is connected to.
#[derive(Clone)]
pub(crate) struct WebviewWebsocketLocation {
    /// The id of the webview that this websocket is connected to
    webview_id: u32,
    server: Arc<Mutex<ServerLocation>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ServerLocation {
    /// The port the websocket is on
    port: u16,
    /// A key that every websocket connection that originates from this application will use to identify itself.
    /// We use this to make sure no external applications can connect to our websocket and receive UI updates.
    client_key: [u8; KEY_SIZE],
    /// The key that the server must respond with for the client to connect to the websocket
    server_key: [u8; KEY_SIZE],
}

/// Start a new server on an available port on localhost. Return the server location and the TCP listener that is bound to the port.
pub(crate) fn start_server() -> (ServerLocation, TcpListener) {
    let client_key = create_secure_key();
    let server_key = create_secure_key();
    let server = TcpListener::bind((IpAddr::from([127, 0, 0, 1]), 0))
        .expect("Failed to bind local TCP listener for edit socket");
    let port = server.local_addr().unwrap().port();
    let location = ServerLocation {
        port,
        client_key,
        server_key,
    };
    (location, server)
}

/// The websocket listener that the webview will connect to in order to receive edits and send requests. There
/// is only one websocket listener per application even if there are multiple windows so we don't use all the
/// open ports.
#[derive(Clone)]
pub(crate) struct EditWebsocket {
    current_location: Arc<Mutex<ServerLocation>>,
    max_webview_id: Arc<AtomicU32>,
    connections: Arc<RwLock<HashMap<u32, WebviewConnectionState>>>,
    server_location: Arc<Notify>,
}

impl EditWebsocket {
    pub(crate) fn start() -> Self {
        let connections = Arc::new(RwLock::new(HashMap::new()));

        let notify = Arc::new(Notify::new());
        let (location, server) = start_server();
        let current_location = Arc::new(Mutex::new(location));

        let connections_ = connections.clone();
        let current_location_ = current_location.clone();
        let notify_ = notify.clone();
        std::thread::spawn(move || {
            Self::accept_loop(notify_, server, current_location_, connections_)
        });

        Self {
            connections,
            max_webview_id: Default::default(),
            current_location,
            server_location: notify,
        }
    }

    /// Accepts incoming websocket connections and handles them in a loop.
    ///
    /// New sockets are accepted and then put in to a new thread to handle the connection.
    /// This is implemented using traditional sync code to allow us to be independent of the async runtime.
    fn accept_loop(
        notify: Arc<Notify>,
        mut server: TcpListener,
        current_location: Arc<Mutex<ServerLocation>>,
        connections: Arc<RwLock<HashMap<u32, WebviewConnectionState>>>,
    ) {
        loop {
            // Accept connections until we hit an error
            while let Ok((stream, _)) = server.accept() {
                Self::handle_connection(stream, current_location.clone(), connections.clone());
            }

            // Switch ports and reconnect on a different port if the server is killed by the OS. This
            // will happen if an IOS device goes to sleep
            //
            // For security, it is important that the keys are also regenerated when the server is restarted.
            // The client may try to reconnect to the old port that is now being used by an attacker who steals the client
            // key and uses it to read the edits from the new port.
            let (location, new_server) = start_server();
            notify.notify_waiters();
            *current_location.lock().unwrap() = location;
            server = new_server;
        }
    }

    fn handle_connection(
        stream: TcpStream,
        server_location: Arc<Mutex<ServerLocation>>,
        connections: Arc<RwLock<HashMap<u32, WebviewConnectionState>>>,
    ) {
        use tungstenite::handshake::server::{Request, Response};

        let current_server_location = { *server_location.lock().unwrap() };
        let hex_encoded_client_key = encode_key_string(&current_server_location.client_key);
        let hex_encoded_server_key = encode_key_string(&current_server_location.server_key);
        let mut location = None;

        #[allow(clippy::result_large_err)]
        let on_request = |req: &Request, res| {
            // Try to parse the webview id and key from the path
            let path = req.uri().path();

            // The path should have two parts `/webview_id/key`
            let mut segments = path.trim_matches('/').split('/');
            let webview_id = segments
                .next()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or_else(|| {
                    Response::builder()
                        .status(400)
                        .body(Some("Bad Request: Invalid webview ID".to_string()))
                        .unwrap()
                })?;
            let key = segments.next().ok_or_else(|| {
                Response::builder()
                    .status(400)
                    .body(Some("Bad Request: Missing key".to_string()))
                    .unwrap()
            })?;

            // Make sure the key matches the expected key.
            // VERY IMPORTANT: We cannot use normal string comparison here because it reveals information
            // about the key based on timing information. Instead we use a constant time comparison method.
            let key_matches: bool =
                subtle::ConstantTimeEq::ct_eq(hex_encoded_client_key.as_ref(), key.as_bytes())
                    .into();
            if !key_matches {
                return Err(Response::builder()
                    .status(403)
                    .body(Some("Forbidden: Invalid key".to_string()))
                    .unwrap());
            }

            location = Some(WebviewWebsocketLocation {
                webview_id,
                server: server_location,
            });

            Ok(res)
        };

        // Accept the websocket connection while reading the path and setting the location
        let mut websocket = match tungstenite::accept_hdr(stream, on_request) {
            Ok(ws) => ws,
            Err(e) => {
                tracing::error!("Error accepting websocket connection: {}", e);
                return;
            }
        };

        // Immediately send the key to authenticate the server
        websocket
            .send(tungstenite::Message::Text(hex_encoded_server_key.into()))
            .unwrap();

        let location = match location {
            Some(loc) => loc,
            None => {
                tracing::error!("WebSocket connection without a valid webview ID");
                return;
            }
        };

        // Handle the websocket connection in a separate thread
        let (edits_outgoing, edits_incoming_rx) = std::sync::mpsc::channel::<MsgPair>();

        let connections_ = connections.clone();
        // Spawn a task to handle the websocket connection
        std::thread::spawn(move || {
            let mut queued_message = None;
            // Wait until there are edits ready to send
            'connection: while let Ok(msg) = edits_incoming_rx.recv() {
                let data = msg.edits.clone();
                queued_message = Some(msg);
                // Send the edits to the webview
                if let Err(e) = websocket.send(tungstenite::Message::Binary(data.into())) {
                    tracing::error!("Error sending edits to webview: {}", e);
                    break 'connection;
                }

                // Wait for the webview to apply the edits
                while let Ok(ws_msg) = websocket.read() {
                    match ws_msg {
                        // We expect the webview to send a binary message when it has applied the edits
                        // This is a signal that we can continue processing
                        tungstenite::Message::Binary(ack) => {
                            // ⛔ THE ACK IS NOT A BOOLEAN — it reports whether
                            // the batch APPLIED, and the difference is the
                            // whole bug this instrument exists for. An empty
                            // payload is the clean case. A leading `1` means
                            // the webview threw partway through, so an unknown
                            // suffix of that batch never reached the DOM.
                            //
                            // Nothing re-sends it. `VirtualDom` diffs against a
                            // model in which those mutations landed, so it will
                            // never emit them again: the subtree they were
                            // building is wrong for the life of the process,
                            // and every later patch aimed at it is addressed to
                            // nodes that were never inserted. That is invisible
                            // from every OS-level instrument and from the app's
                            // own state — the rail that renders no body while
                            // `right_panel_mode` reports the mode changing
                            // correctly was this, twice, and it cost a bisect
                            // across 36 releases before anyone read the DOM.
                            //
                            // Loud, counted, and attributable. It cannot be
                            // repaired from here, but it must never again be
                            // something an agent has to rediscover by walking
                            // the DOM.
                            if ack.first() == Some(&1) {
                                let detail = String::from_utf8_lossy(&ack[1..]).to_string();
                                let faults = EDIT_FAULTS
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                    + 1;
                                tracing::error!(
                                    webview_id = location.webview_id,
                                    faults,
                                    detail = %detail,
                                    "the webview threw while applying an edit batch; the rest of \
                                     that batch never reached the DOM and will never be re-sent \
                                     (this subtree is now permanently stale — restart the GUI to \
                                      clear it)"
                                );
                            }
                            break;
                        }
                        // If the websocket closes, switch back to the pending state and
                        // re-queue the edits that haven't been acknowledged yet
                        tungstenite::Message::Close(_) => {
                            break 'connection;
                        }
                        _ => {}
                    }
                }

                let msg = queued_message.take().expect("Message should be set here");

                // Notify that the edits have been applied
                if msg.response.send(()).is_err() {
                    tracing::debug!("Dropped edits applied notification because the waiter was gone");
                }
            }
            tracing::trace!("Webview {} closed the connection", location.webview_id);
            let mut connection = WebviewConnectionState::default();
            if let Some(msg) = queued_message {
                connection.add_message_pair(msg);
            }
            connections_
                .write()
                .unwrap()
                .insert(location.webview_id, connection);
        });

        let mut connections = connections.write().unwrap();
        match connections.remove(&location.webview_id) {
            // If there are pending edits, send them to the new connection
            Some(WebviewConnectionState::Pending { mut pending }) => {
                while let Some(pair) = pending.pop_front() {
                    _ = edits_outgoing.send(pair);
                }
            }

            // If the webview was already connected, never send edits from the old connection to
            // the new connection. This should never happen
            Some(WebviewConnectionState::Connected { .. }) => {
                tracing::error!(
                    "Webview {} was already connected. Rejecting new connection.",
                    location.webview_id
                );
                return;
            }

            None => {}
        }

        connections.insert(
            location.webview_id,
            WebviewConnectionState::Connected { edits_outgoing },
        );
    }

    pub(crate) fn create_queue(&self) -> WryQueue {
        let webview_id = self
            .max_webview_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let server = self.current_location.clone();
        let server_location = self.server_location.clone();
        WryQueue {
            inner: Rc::new(RefCell::new(WryQueueInner {
                server_location_changed: server_location.clone(),
                server_location_changed_future: owned_notify_future(server_location),
                location: WebviewWebsocketLocation { webview_id, server },
                websocket: self.clone(),
                edits_in_progress: None,
                edits_deadline: None,
                abandoned_edits: VecDeque::new(),
                unacked_batches: 0,
                last_ack_at: std::time::Instant::now(),
                resync_requested: false,
                mutation_state: MutationState::default(),
            })),
        }
    }

    fn send_edits(&mut self, webview: u32, edits: Vec<u8>) -> oneshot::Receiver<()> {
        let mut connections_mut = self.connections.write().unwrap();
        let connection = connections_mut.entry(webview).or_default();
        connection.add_message(edits)
    }
}

/// The state of a webview websocket connection. This may be pending while the webview is booting.
/// If it is, we queue up edits until the webview is ready to receive them.
enum WebviewConnectionState {
    Pending {
        pending: VecDeque<MsgPair>,
    },
    Connected {
        edits_outgoing: std::sync::mpsc::Sender<MsgPair>,
    },
}

impl Default for WebviewConnectionState {
    fn default() -> Self {
        WebviewConnectionState::Pending {
            pending: VecDeque::new(),
        }
    }
}

impl WebviewConnectionState {
    /// Add a message to the active connection or queue and return a receiver that will be resolved
    /// when the webview has applied the edits.
    fn add_message(&mut self, edits: Vec<u8>) -> oneshot::Receiver<()> {
        let (response_sender, response_receiver) = oneshot::channel();
        let pair = MsgPair {
            edits,
            response: response_sender,
        };
        self.add_message_pair(pair);
        response_receiver
    }

    /// Add a message pair to the connection state. The receiver in the message pair will be resolved
    /// when the webview has applied the edits.
    fn add_message_pair(&mut self, pair: MsgPair) {
        match self {
            WebviewConnectionState::Pending { pending: queue } => {
                queue.push_back(pair);
            }
            WebviewConnectionState::Connected { edits_outgoing } => {
                _ = edits_outgoing.send(pair);
            }
        }
    }
}

struct MsgPair {
    edits: Vec<u8>,
    response: oneshot::Sender<()>,
}

const KEY_SIZE: usize = 256;
type EncodedKey = [u8; KEY_SIZE];

/// Base64 encode the key to a string to be used in the websocket URL.
fn encode_key_string(key: &EncodedKey) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE, key)
}

/// Create a secure key for the websocket connection.
/// Returns the key as a byte array and a hex-encoded string representation of the key.
fn create_secure_key() -> EncodedKey {
    // Helper function to assert that the RNG is a CryptoRng - make sure we use a secure RNG
    fn assert_crypto_random<R: rand::CryptoRng>(val: R) -> R {
        val
    }

    let mut secure_rng = assert_crypto_random(rand::rngs::StdRng::from_os_rng());
    let mut expected_key: EncodedKey = [0u8; KEY_SIZE];
    secure_rng.fill_bytes(&mut expected_key);
    expected_key
}

#[test]
fn test_key_encoding_length() {
    let mut rand = rand::rngs::StdRng::from_os_rng();
    for _ in 0..100 {
        let mut key: EncodedKey = [0u8; KEY_SIZE];
        rand.fill_bytes(&mut key);
        let encoded = encode_key_string(&key);
        // The encoded key length should be the same regardless of the value of the key
        assert_eq!(encoded.len(), 344);
    }
}

// Take an Arc<Notify> and create a future that waits for the notify to be triggered.
fn owned_notify_future(notify: Arc<Notify>) -> Pin<Box<dyn Future<Output = ()>>> {
    let mut notify_owned = Box::pin(async move {
        let notified = notify.notified();

        // The future should be after this statement once it is polled bellow
        tokio::task::yield_now().await;
        notified.await;
    });

    // Start tracking notify before the output future is polled
    _ = (&mut notify_owned).now_or_never();
    notify_owned
}

#[cfg(test)]
mod edit_flush_recovery_tests {
    use super::*;

    // The ordinary case: one slow frame is waited for, as designed.
    #[test]
    fn a_single_unanswered_batch_still_gates() {
        assert_eq!(edit_flush_recovery(0, 0), EditFlushRecovery::Gate);
        assert_eq!(edit_flush_recovery(1, 1_500), EditFlushRecovery::Gate);
        assert_eq!(
            edit_flush_recovery(EDIT_FLUSH_GATE_BYPASS_AFTER - 1, 5_000),
            EditFlushRecovery::Gate
        );
    }

    // The freeze the owner feels is the REPEAT, not the first wait.
    #[test]
    fn a_webview_several_acks_behind_stops_freezing_the_vdom() {
        assert_eq!(
            edit_flush_recovery(EDIT_FLUSH_GATE_BYPASS_AFTER, 6_000),
            EditFlushRecovery::BypassGate
        );
        assert_eq!(edit_flush_recovery(11, 19_000), EditFlushRecovery::BypassGate);
    }

    // ⛔ THE ONE THAT MATTERS: a reload is a full remount, so a webview that is
    // merely SLOW must never reach it. Both conditions are required, and the
    // time floor is what stops a freely-running VirtualDom from producing a
    // dozen batches in a second and calling the page dead.
    #[test]
    fn only_a_silent_ack_plane_earns_a_reload() {
        assert_eq!(
            edit_flush_recovery(EDIT_ACK_DEAD_BATCHES, EDIT_ACK_DEAD_MS),
            EditFlushRecovery::ReloadWebview
        );
        assert_eq!(
            edit_flush_recovery(EDIT_ACK_DEAD_BATCHES, EDIT_ACK_DEAD_MS - 1),
            EditFlushRecovery::BypassGate,
            "a burst of batches inside the window is not a dead webview"
        );
        assert_eq!(
            edit_flush_recovery(EDIT_ACK_DEAD_BATCHES - 1, 10 * EDIT_ACK_DEAD_MS),
            EditFlushRecovery::BypassGate,
            "a long quiet stretch with few batches is not a dead webview either"
        );
        // And the reset that makes a late ack decisive: the caller zeroes both
        // inputs on any acknowledgement, so the ladder drops straight back to
        // the bottom rung.
        assert_eq!(edit_flush_recovery(0, 0), EditFlushRecovery::Gate);
    }
}
