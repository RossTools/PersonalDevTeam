//! Where a live conversation's outputs go, with backpressure: a reader waits ([`EventSink::ready`]) while the
//! conversation's queue is full, so a slow store pauses reading the agent's pipe instead of growing pdtd's memory.
//!
//! Sending never waits (the adapters send from inside their mappers' locks); the bound is kept by the readers, which
//! wait before the next frame. A frame may send a few outputs, so the queue can pass its bound by those.

use super::AdapterOutput;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;
use tokio::sync::mpsc;

/// How many outputs wait for a conversation's pump before its reader pauses.
pub const EVENT_QUEUE: usize = 1024;

struct Shared {
    queued: AtomicUsize,
    cap: usize,
    drained: Notify,
}

/// Nobody reads the stream any more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

/// The adapter's end: send outputs, and wait for room before reading on.
#[derive(Clone)]
pub struct EventSink {
    tx: mpsc::UnboundedSender<AdapterOutput>,
    shared: Arc<Shared>,
}

/// The conversation's end: the outputs, in order.
pub struct EventStream {
    rx: mpsc::UnboundedReceiver<AdapterOutput>,
    shared: Arc<Shared>,
}

/// A sink and its stream; the reader pauses once `cap` outputs wait.
pub fn event_channel(cap: usize) -> (EventSink, EventStream) {
    let (tx, rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared { queued: AtomicUsize::new(0), cap: cap.max(1), drained: Notify::new() });
    (EventSink { tx, shared: shared.clone() }, EventStream { rx, shared })
}

impl EventSink {
    /// Queues an output; never waits. Fails once the stream is gone.
    pub fn send(&self, out: AdapterOutput) -> Result<(), Closed> {
        self.shared.queued.fetch_add(1, Ordering::SeqCst);
        self.tx.send(out).map_err(|_| {
            self.shared.queued.fetch_sub(1, Ordering::SeqCst);
            Closed
        })
    }

    /// Waits until the queue has room (or nobody reads it any more): call before reading the next frame.
    pub async fn ready(&self) {
        loop {
            let drained = self.shared.drained.notified();
            tokio::pin!(drained);
            drained.as_mut().enable();
            if self.shared.queued.load(Ordering::SeqCst) < self.shared.cap || self.tx.is_closed() {
                return;
            }
            // `is_closed` changes without a notification: look again now and then.
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), drained).await;
        }
    }

    /// How many outputs wait.
    pub fn queued(&self) -> usize {
        self.shared.queued.load(Ordering::SeqCst)
    }
}

impl EventStream {
    pub async fn recv(&mut self) -> Option<AdapterOutput> {
        let out = self.rx.recv().await;
        if out.is_some() {
            self.took();
        }
        out
    }

    pub fn try_recv(&mut self) -> Result<AdapterOutput, mpsc::error::TryRecvError> {
        let out = self.rx.try_recv();
        if out.is_ok() {
            self.took();
        }
        out
    }

    fn took(&self) {
        let before = self.shared.queued.fetch_sub(1, Ordering::SeqCst);
        if before <= self.shared.cap {
            self.shared.drained.notify_waiters();
        }
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        // A reader waiting for room carries on (its sends now fail).
        self.rx.close();
        self.shared.drained.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn out(n: i32) -> AdapterOutput {
        AdapterOutput::ExternalId(n.to_string())
    }

    #[tokio::test]
    async fn ready_waits_while_the_queue_is_full() {
        let (sink, mut stream) = event_channel(2);
        sink.send(out(1)).unwrap();
        tokio::time::timeout(Duration::from_millis(50), sink.ready()).await.expect("room for one more");
        sink.send(out(2)).unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(50), sink.ready()).await.is_err(), "full: it waits");

        let waiting = tokio::spawn({
            let sink = sink.clone();
            async move { sink.ready().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(stream.recv().await, Some(out(1)));
        tokio::time::timeout(Duration::from_secs(1), waiting).await.expect("taking one makes room").unwrap();
        assert_eq!(sink.queued(), 1);
    }

    #[tokio::test]
    async fn sends_past_the_bound_are_kept_in_order() {
        let (sink, mut stream) = event_channel(1);
        for n in 0..5 {
            sink.send(out(n)).unwrap();
        }
        for n in 0..5 {
            assert_eq!(stream.recv().await, Some(out(n)));
        }
        assert_eq!(sink.queued(), 0);
    }

    #[tokio::test]
    async fn a_dropped_stream_releases_a_waiting_reader() {
        let (sink, stream) = event_channel(1);
        sink.send(out(1)).unwrap();
        let waiting = tokio::spawn({
            let sink = sink.clone();
            async move { sink.ready().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(stream);
        tokio::time::timeout(Duration::from_secs(1), waiting).await.expect("released").unwrap();
        assert!(sink.send(out(2)).is_err());
        tokio::time::timeout(Duration::from_millis(50), sink.ready()).await.expect("never waits once nobody reads");
    }
}
