use oxrdf::Quad;
use tokio::sync::broadcast;

/// Lightweight event sent from sync server threads into the async CDC world.
#[derive(Debug, Clone)]
pub struct ChangeEvent {
    pub id: u64,
    pub timestamp: String,
    pub operation: String,
    pub inserted: Vec<Quad>,
    pub removed: Vec<Quad>,
}

/// Wraps a [`broadcast::Sender<ChangeEvent>`].
///
/// Clone-able, Send+Sync, usable from synchronous code.
/// `send()` is non-blocking (lock-free ring buffer).
#[derive(Clone)]
pub struct ChangeEventSender {
    inner: broadcast::Sender<ChangeEvent>,
}

impl ChangeEventSender {
    /// Send a change event. Returns `Ok(receiver_count)` or `Err` if no receivers.
    /// Fire-and-forget: callers should ignore the error.
    pub fn send(
        &self,
        event: ChangeEvent,
    ) -> Result<usize, broadcast::error::SendError<ChangeEvent>> {
        self.inner.send(event)
    }

    /// Create a new receiver for this sender's broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<ChangeEvent> {
        self.inner.subscribe()
    }
}

/// Create a new broadcast channel pair with the given capacity.
pub fn new_broadcast(capacity: usize) -> (ChangeEventSender, broadcast::Receiver<ChangeEvent>) {
    let (tx, rx) = broadcast::channel(capacity);
    (ChangeEventSender { inner: tx }, rx)
}
