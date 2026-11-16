//! Bounded, owned PCM buffers for APIs that copy caller memory and block for
//! queue space (such as the FMOD Java `AudioDevice` path).
//!
//! Producers transfer ownership of each `Vec<u8>`. The realtime consumer uses
//! `try_copy`, which never waits for the producer's mutex and does not allocate
//! or free memory. Completed buffers move to a preallocated reclaim queue; a
//! non-realtime producer drains them with `take_reclaimed`. Closing the queue
//! releases pending buffers and wakes producers blocked on capacity.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, TryLockError};
use std::time::{Duration, Instant};

struct State {
    buffers: VecDeque<Vec<u8>>,
    reclaimed: VecDeque<Vec<u8>>,
    front_offset: usize,
    closed: bool,
}

/// A bounded queue of owned PCM chunks.
pub struct OwnedPcmQueue {
    capacity: usize,
    state: Mutex<State>,
    space: Condvar,
}

/// Why a producer could not submit a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    /// The queue was closed before the chunk could be accepted.
    Closed,
    /// The queue remained full until the producer's deadline.
    TimedOut,
    /// Empty chunks are not useful to a PCM sink.
    Empty,
}

impl OwnedPcmQueue {
    /// Create a queue with a fixed maximum number of pending chunks.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "PCM queue capacity must be nonzero");
        Self {
            capacity,
            state: Mutex::new(State {
                buffers: VecDeque::with_capacity(capacity),
                reclaimed: VecDeque::with_capacity(capacity),
                front_offset: 0,
                closed: false,
            }),
            space: Condvar::new(),
        }
    }

    /// Transfer a chunk to the queue, waiting at most `timeout` for room.
    ///
    /// A successful push transfers ownership. On failure the caller retains
    /// ownership of `buffer` through the returned error tuple.
    pub fn push(&self, buffer: Vec<u8>, timeout: Duration) -> Result<(), (PushError, Vec<u8>)> {
        if buffer.is_empty() {
            return Err((PushError::Empty, buffer));
        }
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if state.closed {
                return Err((PushError::Closed, buffer));
            }
            if state.buffers.len() + state.reclaimed.len() < self.capacity {
                state.buffers.push_back(buffer);
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err((PushError::TimedOut, buffer));
            }
            let wait = deadline.saturating_duration_since(now);
            let (next, result) = self
                .space
                .wait_timeout(state, wait)
                .unwrap_or_else(|p| p.into_inner());
            state = next;
            if result.timed_out() && state.buffers.len() + state.reclaimed.len() >= self.capacity {
                return Err((PushError::TimedOut, buffer));
            }
        }
    }

    /// Copy queued PCM into `destination` without waiting, allocating, or
    /// freeing. A completed chunk is moved to the producer-side reclaim queue.
    /// If the queue mutex is busy, returns zero for this callback.
    pub fn try_copy(&self, destination: &mut [u8]) -> usize {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return 0,
        };
        let mut copied = 0;
        while copied < destination.len() {
            let Some(front) = state.buffers.front() else {
                break;
            };
            let front_len = front.len();
            let offset = state.front_offset;
            let remaining = front_len - offset;
            let count = remaining.min(destination.len() - copied);
            destination[copied..copied + count].copy_from_slice(&front[offset..offset + count]);
            copied += count;
            state.front_offset += count;
            if state.front_offset == front_len {
                let completed = state.buffers.pop_front().expect("front was present");
                state.reclaimed.push_back(completed);
                state.front_offset = 0;
            }
        }
        copied
    }

    /// Take a completed buffer for reuse or deallocation on a non-realtime
    /// producer thread. Taking it releases one queue slot.
    pub fn take_reclaimed(&self) -> Option<Vec<u8>> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let buffer = state.reclaimed.pop_front();
        if buffer.is_some() {
            self.space.notify_one();
        }
        buffer
    }

    /// Close the queue, discard pending chunks, and wake all blocked producers.
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.closed = true;
        state.buffers.clear();
        state.reclaimed.clear();
        state.front_offset = 0;
        self.space.notify_all();
    }

    /// Number of chunks currently waiting to be consumed.
    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .buffers
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn queue_is_bounded_and_reclaims_chunks_in_fifo_order() {
        let queue = OwnedPcmQueue::new(1);
        queue.push(vec![1, 2], Duration::ZERO).unwrap();
        let (error, rejected) = queue.push(vec![3], Duration::ZERO).unwrap_err();
        assert_eq!(error, PushError::TimedOut);
        assert_eq!(rejected, [3]);
        let mut output = [0; 2];
        assert_eq!(queue.try_copy(&mut output), 2);
        assert_eq!(output, [1, 2]);
        assert!(queue.is_empty());
        let (error, still_owned) = queue.push(vec![5], Duration::ZERO).unwrap_err();
        assert_eq!(error, PushError::TimedOut);
        assert_eq!(still_owned, [5]);
        assert_eq!(queue.take_reclaimed().unwrap(), [1, 2]);
        queue.push(vec![4], Duration::ZERO).unwrap();
        assert_eq!(queue.try_copy(&mut output[..1]), 1);
        assert_eq!(queue.take_reclaimed().unwrap(), [4]);
    }

    #[test]
    fn partial_copy_retains_chunk_until_its_last_byte_is_consumed() {
        let queue = OwnedPcmQueue::new(2);
        queue.push(vec![1, 2, 3], Duration::ZERO).unwrap();
        queue.push(vec![4, 5], Duration::ZERO).unwrap();
        let mut first = [0; 2];
        assert_eq!(queue.try_copy(&mut first), 2);
        assert_eq!(first, [1, 2]);
        assert!(queue.take_reclaimed().is_none());
        let mut rest = [0; 3];
        assert_eq!(queue.try_copy(&mut rest), 3);
        assert_eq!(rest, [3, 4, 5]);
        assert_eq!(queue.take_reclaimed().unwrap(), [1, 2, 3]);
        assert_eq!(queue.take_reclaimed().unwrap(), [4, 5]);
    }

    #[test]
    fn close_cancels_a_producer_waiting_for_capacity() {
        let queue = Arc::new(OwnedPcmQueue::new(1));
        queue.push(vec![1], Duration::ZERO).unwrap();
        let producer_queue = Arc::clone(&queue);
        let producer = thread::spawn(move || producer_queue.push(vec![2], Duration::from_secs(5)));
        thread::sleep(Duration::from_millis(20));
        queue.close();
        let (error, bytes) = producer.join().unwrap().unwrap_err();
        assert_eq!(error, PushError::Closed);
        assert_eq!(bytes, [2]);
        assert!(queue.is_empty());
        let mut output = [0; 1];
        assert_eq!(queue.try_copy(&mut output), 0);
    }

    #[test]
    fn producer_timeout_returns_ownership_without_dropping_data() {
        let queue = OwnedPcmQueue::new(1);
        queue.push(vec![1], Duration::ZERO).unwrap();
        let (error, bytes) = queue
            .push(vec![8, 9], Duration::from_millis(1))
            .unwrap_err();
        assert_eq!(error, PushError::TimedOut);
        assert_eq!(bytes, [8, 9]);
    }
}
