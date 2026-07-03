// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `EgressBus`: an `Interconnect` that drains tapped items each sim step and
//! hands encoded payloads to a transport worker thread. Enqueue-only on the sim
//! side; all blocking network I/O happens on the worker thread.

use crate::network::egress::encoding::encode;
use crate::network::egress::transport::EgressTransport;
use crate::network::egress::{BufferPolicy, EgressItem, EncodingKind};
use crate::network::{CanFrame, Interconnect};
use crate::SimResult;
use std::collections::VecDeque;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

/// Bound on the worker payload queue. Independent of the item BufferPolicy.
const WORKER_QUEUE_DEPTH: usize = 64;

pub struct EgressBus {
    rx: Receiver<EgressItem>,
    can_sources: Vec<Receiver<CanFrame>>,
    buffer: VecDeque<EgressItem>,
    /// A payload already encoded but not yet accepted by the worker (endpoint
    /// backed up). Retried before encoding new items, so each item is encoded
    /// exactly once regardless of backpressure.
    pending: Option<Vec<u8>>,
    policy: BufferPolicy,
    encoding: EncodingKind,
    dropped: u64,
    /// Value of `dropped` at the last emitted warning, so we log on change
    /// rather than every tick.
    warned_dropped: u64,
    worker_tx: Option<SyncSender<Vec<u8>>>,
    worker: Option<JoinHandle<()>>,
}

impl EgressBus {
    pub fn new(
        rx: Receiver<EgressItem>,
        encoding: EncodingKind,
        policy: BufferPolicy,
        mut transport: Box<dyn EgressTransport>,
    ) -> Self {
        let (worker_tx, worker_rx) = sync_channel::<Vec<u8>>(WORKER_QUEUE_DEPTH);
        let worker = std::thread::spawn(move || {
            while let Ok(payload) = worker_rx.recv() {
                let _ = transport.send(&payload);
            }
        });
        Self {
            rx,
            can_sources: Vec::new(),
            buffer: VecDeque::new(),
            pending: None,
            policy,
            encoding,
            dropped: 0,
            warned_dropped: 0,
            worker_tx: Some(worker_tx),
            worker: Some(worker),
        }
    }

    /// Register a CAN frame receiver (e.g. from `CanBus::attach`).
    pub fn add_can_source(&mut self, rx: Receiver<CanFrame>) {
        self.can_sources.push(rx);
    }

    /// Items discarded by the bounded buffer since construction.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    fn drain_sources(&mut self) {
        while let Ok(item) = self.rx.try_recv() {
            self.buffer.push_back(item);
        }
        for src in &self.can_sources {
            while let Ok(frame) = src.try_recv() {
                self.buffer.push_back(EgressItem::Frame(frame));
            }
        }
        while self.buffer.len() > self.policy.max {
            self.buffer.pop_front();
            self.dropped += 1;
        }
        // Surface backpressure loss once per burst instead of dropping silently.
        if self.dropped > self.warned_dropped {
            tracing::warn!(
                dropped = self.dropped,
                "egress endpoint can't keep up; dropped oldest buffered items"
            );
            self.warned_dropped = self.dropped;
        }
    }

    /// Try to hand `payload` to the worker. On a full queue the payload is
    /// stashed in `pending` (never re-encoded); on disconnect it is discarded.
    fn try_send(&mut self, payload: Vec<u8>) {
        let Some(tx) = &self.worker_tx else { return };
        match tx.try_send(payload) {
            Ok(()) => {}
            Err(TrySendError::Full(p)) => self.pending = Some(p),
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

impl Interconnect for EgressBus {
    fn tick(&mut self) -> SimResult<()> {
        self.drain_sources();
        // Flush a previously-rejected payload before encoding anything new, so
        // ordering holds and the worker queue drains in FIFO order.
        if let Some(pending) = self.pending.take() {
            self.try_send(pending);
            if self.pending.is_some() {
                return Ok(()); // still backed up; don't pile on more work
            }
        }
        if self.buffer.is_empty() {
            return Ok(());
        }
        let items: Vec<EgressItem> = self.buffer.drain(..).collect();
        let payload = encode(self.encoding, &items);
        if !payload.is_empty() {
            self.try_send(payload);
        }
        Ok(())
    }
}

impl Drop for EgressBus {
    fn drop(&mut self) {
        // Close the channel so the worker loop exits, then join.
        self.worker_tx.take();
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::egress::transport::MemoryTransport;
    use crate::network::egress::{BufferPolicy, EgressItem, EncodingKind};
    use crate::network::Interconnect;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    fn wait_for<F: Fn() -> bool>(f: F) {
        let start = Instant::now();
        while !f() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn bus_forwards_encoded_bytes_to_transport() {
        let (tx, rx) = channel();
        let transport = MemoryTransport::new();
        let handle = transport.handle();
        let mut bus = EgressBus::new(
            rx,
            EncodingKind::Raw,
            BufferPolicy::default(),
            Box::new(transport),
        );
        tx.send(EgressItem::Byte(b'h')).unwrap();
        tx.send(EgressItem::Byte(b'i')).unwrap();
        bus.tick().unwrap();
        wait_for(|| !handle.lock().unwrap().is_empty());
        let got = handle.lock().unwrap();
        assert_eq!(got.concat(), b"hi".to_vec());
    }

    #[test]
    fn bounded_buffer_drops_oldest_and_counts() {
        // Never tick, so nothing is consumed: overflow must drop oldest.
        let (tx, rx) = channel();
        let transport = MemoryTransport::new();
        let mut bus = EgressBus::new(
            rx,
            EncodingKind::Raw,
            BufferPolicy { max: 2 },
            Box::new(transport),
        );
        for b in 0..5u8 {
            tx.send(EgressItem::Byte(b)).unwrap();
        }
        bus.tick().unwrap();
        // 5 items in, buffer cap 2 → 3 dropped.
        assert_eq!(bus.dropped(), 3);
    }
}
