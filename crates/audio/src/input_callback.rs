//! Cancellable bridge from a pull-style capture source to an AAudio input
//! callback. Runs on its own non-realtime thread, so it may wait for a whole
//! burst without ever blocking the host audio thread.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

/// Fills `dst` with whatever is available and returns the byte count. Must not block.
pub type ReadFn = Box<dyn FnMut(&mut [u8]) -> usize + Send>;
/// Receives one whole burst. Return `false` to stop (AAUDIO_CALLBACK_RESULT_STOP).
pub type DataFn = Box<dyn FnMut(&mut [u8], u32) -> bool + Send>;
/// Runs exactly once when the worker exits, on every path including panics.
/// The argument is `true` when the data callback asked to stop.
pub type ExitFn = Box<dyn FnOnce(bool) + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    InvalidArguments,
    FromCallbackThread,
    AlreadyRunning,
    SpawnFailed,
}

thread_local! {
    static CURRENT: Cell<usize> = const { Cell::new(0) };
}

struct Shared {
    cancelled: AtomicBool,
    running: AtomicBool,
    wait: Mutex<()>,
    wake: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn id(shared: &Arc<Shared>) -> usize {
    Arc::as_ptr(shared) as usize
}

impl Shared {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        // Taking the lock orders this against a worker that is between its
        // cancelled check and its wait, so the wakeup cannot be lost.
        drop(lock(&self.wait));
        self.wake.notify_all();
    }

    fn idle_wait(&self) {
        let guard = lock(&self.wait);
        let _ = self
            .wake
            .wait_timeout_while(guard, Duration::from_millis(1), |_| {
                !self.cancelled.load(Ordering::Acquire)
            });
    }
}

/// Cheap clone that can be captured by the data callback: cancel without
/// joining, and ask whether the caller is the callback thread.
#[derive(Clone)]
pub struct InputCallbackHandle(Arc<Shared>);

impl InputCallbackHandle {
    /// Safe from inside the data callback because it never joins.
    pub fn cancel(&self) {
        self.0.cancel();
    }
    pub fn is_callback_thread(&self) -> bool {
        CURRENT.with(|c| c.get()) == id(&self.0)
    }
    pub fn is_running(&self) -> bool {
        self.0.running.load(Ordering::Acquire)
    }
}

pub struct InputCallbackDriver {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Default for InputCallbackDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl InputCallbackDriver {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                cancelled: AtomicBool::new(true),
                running: AtomicBool::new(false),
                wait: Mutex::new(()),
                wake: Condvar::new(),
            }),
            worker: Mutex::new(None),
        }
    }

    pub fn handle(&self) -> InputCallbackHandle {
        InputCallbackHandle(Arc::clone(&self.shared))
    }

    pub fn start(
        &self,
        bytes_per_frame: u32,
        frames_per_callback: u32,
        read: ReadFn,
        data: DataFn,
        exited: ExitFn,
    ) -> Result<(), StartError> {
        if bytes_per_frame == 0 || frames_per_callback == 0 {
            return Err(StartError::InvalidArguments);
        }
        let len = (bytes_per_frame as usize)
            .checked_mul(frames_per_callback as usize)
            .ok_or(StartError::InvalidArguments)?;
        if self.on_callback_thread() {
            return Err(StartError::FromCallbackThread);
        }
        if self.shared.running.load(Ordering::Acquire) {
            return Err(StartError::AlreadyRunning);
        }

        // Reap a previous worker that has already stopped, so its exit hook
        // has finished before the new run begins.
        self.join();
        self.shared.cancelled.store(false, Ordering::Release);
        self.shared.running.store(true, Ordering::Release);

        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("roblox-aaudio-input".into())
            .spawn(move || worker(shared, len, frames_per_callback, read, data, exited));
        match spawned {
            Ok(handle) => {
                *lock(&self.worker) = Some(handle);
                Ok(())
            }
            Err(_) => {
                self.shared.cancelled.store(true, Ordering::Release);
                self.shared.running.store(false, Ordering::Release);
                Err(StartError::SpawnFailed)
            }
        }
    }

    pub fn cancel(&self) {
        self.shared.cancel();
    }

    /// Waits for the worker. A no-op when called from the callback thread.
    pub fn join(&self) {
        if self.on_callback_thread() {
            return;
        }
        let handle = lock(&self.worker).take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    pub fn is_callback_thread(&self) -> bool {
        self.on_callback_thread()
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }

    fn on_callback_thread(&self) -> bool {
        CURRENT.with(|c| c.get()) == id(&self.shared)
    }
}

impl Drop for InputCallbackDriver {
    fn drop(&mut self) {
        self.cancel();
        self.join();
    }
}

/// Runs the exit hook on every way out of the worker, panics included. The
/// microphone must close however the loop ends.
struct ExitGuard<'a> {
    shared: &'a Arc<Shared>,
    exited: Option<ExitFn>,
    callback_stopped: bool,
}

impl Drop for ExitGuard<'_> {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Release);
        if let Some(exited) = self.exited.take() {
            exited(self.callback_stopped);
        }
        CURRENT.with(|c| c.set(0));
    }
}

fn worker(
    shared: Arc<Shared>,
    len: usize,
    frames: u32,
    mut read: ReadFn,
    mut data: DataFn,
    exited: ExitFn,
) {
    CURRENT.with(|c| c.set(id(&shared)));
    let mut guard = ExitGuard {
        shared: &shared,
        exited: Some(exited),
        callback_stopped: false,
    };
    let mut buffer = vec![0u8; len];
    let mut filled = 0usize;

    while !shared.cancelled.load(Ordering::Acquire) {
        let got = read(&mut buffer[filled..]);
        if got > len - filled {
            // A source claiming more than it was given is broken; stop.
            shared.cancelled.store(true, Ordering::Release);
            break;
        }
        filled += got;

        if filled == len {
            if !data(&mut buffer, frames) {
                guard.callback_stopped = true;
                break;
            }
            filled = 0;
            continue;
        }
        if got == 0 {
            shared.idle_wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicUsize};
    use std::sync::mpsc;
    use std::time::Instant;

    const T: Duration = Duration::from_secs(2);

    struct Src {
        offset: Arc<AtomicUsize>,
        reads: Arc<AtomicU32>,
    }

    fn source(bytes: Vec<u8>, largest: usize) -> (ReadFn, Src) {
        let offset = Arc::new(AtomicUsize::new(0));
        let reads = Arc::new(AtomicU32::new(0));
        let (o, r) = (offset.clone(), reads.clone());
        let f: ReadFn = Box::new(move |dst| {
            r.fetch_add(1, Ordering::SeqCst);
            let off = o.load(Ordering::SeqCst);
            if off >= bytes.len() {
                return 0;
            }
            let n = (bytes.len() - off).min(dst.len()).min(largest);
            dst[..n].copy_from_slice(&bytes[off..off + n]);
            o.store(off + n, Ordering::SeqCst);
            n
        });
        (f, Src { offset, reads })
    }

    struct Probe {
        calls: Arc<AtomicU32>,
        delivered: mpsc::Receiver<(Vec<u8>, u32, bool)>,
        exit: mpsc::Receiver<bool>,
    }

    fn start(
        driver: &InputCallbackDriver,
        read: ReadFn,
        keep_going: bool,
        cancel_inside: bool,
    ) -> Probe {
        let (dtx, drx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let h = driver.handle();
        let data: DataFn = Box::new(move |buf, frames| {
            c.fetch_add(1, Ordering::SeqCst);
            dtx.send((buf.to_vec(), frames, h.is_callback_thread()))
                .ok();
            if cancel_inside {
                h.cancel();
            }
            keep_going
        });
        let exited: ExitFn = Box::new(move |stopped| {
            etx.send(stopped).ok();
        });
        driver.start(2, 480, read, data, exited).expect("start");
        Probe {
            calls,
            delivered: drx,
            exit: erx,
        }
    }

    #[test]
    fn fragmented_s16_bytes_reach_one_whole_frame_callback_unchanged() {
        let bytes: Vec<u8> = (0..960usize).map(|i| ((i * 29) & 0xff) as u8).collect();
        let (read, _src) = source(bytes.clone(), 37);
        let driver = InputCallbackDriver::new();
        let p = start(&driver, read, false, false);
        assert_eq!(p.exit.recv_timeout(T), Ok(true));
        driver.join();

        let (got, frames, on_cb) = p.delivered.try_recv().unwrap();
        assert_eq!(p.calls.load(Ordering::SeqCst), 1);
        assert_eq!(frames, 480);
        assert_eq!(got, bytes);
        assert!(on_cb);
        assert!(!driver.is_running());
    }

    #[test]
    fn callback_stop_delivers_once_then_exits() {
        let (read, src) = source(vec![0x5a; 480 * 2 * 3], usize::MAX);
        let driver = InputCallbackDriver::new();
        let p = start(&driver, read, false, false);
        assert_eq!(p.exit.recv_timeout(T), Ok(true));
        driver.join();
        assert_eq!(p.calls.load(Ordering::SeqCst), 1);
        assert_eq!(src.offset.load(Ordering::SeqCst), 480 * 2);
    }

    #[test]
    fn idle_cancellation_wakes_without_delivering_partial_audio() {
        let (read, src) = source(Vec::new(), usize::MAX);
        let driver = InputCallbackDriver::new();
        let p = start(&driver, read, true, false);

        let deadline = Instant::now() + T;
        while src.reads.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(src.reads.load(Ordering::SeqCst) != 0);

        let before = Instant::now();
        driver.cancel();
        driver.join();
        assert!(before.elapsed() < Duration::from_millis(250));
        assert_eq!(p.calls.load(Ordering::SeqCst), 0);
        assert_eq!(p.exit.recv_timeout(T), Ok(false));
        assert!(!driver.is_running());
    }

    #[test]
    fn callback_thread_can_request_cancellation_without_self_joining() {
        let (read, _src) = source(vec![0x37; 480 * 2], usize::MAX);
        let driver = InputCallbackDriver::new();
        let p = start(&driver, read, true, true);
        assert_eq!(p.exit.recv_timeout(T), Ok(false));
        driver.join();
        let (_, _, on_cb) = p.delivered.try_recv().unwrap();
        assert!(on_cb);
        assert_eq!(p.calls.load(Ordering::SeqCst), 1);
        assert!(!driver.is_running());
    }

    #[test]
    fn a_cancelled_driver_can_start_again() {
        let driver = InputCallbackDriver::new();
        let (idle, _s) = source(Vec::new(), usize::MAX);
        let first = start(&driver, idle, true, false);
        driver.cancel();
        driver.join();

        let (read, _s2) = source(vec![0x42; 480 * 2], usize::MAX);
        let second = start(&driver, read, false, false);
        assert_eq!(second.exit.recv_timeout(T), Ok(true));
        driver.join();

        assert_eq!(first.calls.load(Ordering::SeqCst), 0);
        assert_eq!(first.exit.recv_timeout(T), Ok(false));
        assert_eq!(second.calls.load(Ordering::SeqCst), 1);
    }
}
