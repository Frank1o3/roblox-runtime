//! Android's per-thread ALooper API, backed by epoll and eventfd.

use std::cell::RefCell;
use std::ffi::{c_int, c_void};
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicUsize, Ordering};

const POLL_WAKE: c_int = -1;
const POLL_CALLBACK: c_int = -2;
const POLL_TIMEOUT: c_int = -3;
const POLL_ERROR: c_int = -4;
const EVENT_INPUT: c_int = 1;
const EVENT_OUTPUT: c_int = 2;
const EVENT_ERROR: c_int = 4;
const EVENT_HANGUP: c_int = 8;
const EPOLLIN: u32 = 1;
const EPOLLOUT: u32 = 4;
const EPOLLERR: u32 = 8;
const EPOLLHUP: u32 = 16;
const EPOLL_CTL_ADD: c_int = 1;
const EPOLL_CTL_DEL: c_int = 2;

#[cfg_attr(target_arch = "x86_64", repr(C, packed))]
#[cfg_attr(not(target_arch = "x86_64"), repr(C))]
#[derive(Clone, Copy)]
struct EpollEvent {
    events: u32,
    data: u64,
}

unsafe extern "C" {
    fn epoll_create1(flags: c_int) -> c_int;
    fn epoll_ctl(epfd: c_int, op: c_int, fd: c_int, event: *mut EpollEvent) -> c_int;
    fn epoll_wait(epfd: c_int, events: *mut EpollEvent, maxevents: c_int, timeout: c_int) -> c_int;
    fn eventfd(initval: u32, flags: c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn close(fd: c_int) -> c_int;
}

type Callback = extern "C" fn(c_int, c_int, *mut c_void) -> c_int;

struct Registration {
    fd: RawFd,
    ident: c_int,
    callback: Option<Callback>,
    data: *mut c_void,
}

struct Looper {
    epoll: RawFd,
    wake_fd: RawFd,
    refs: AtomicUsize,
    registrations: RefCell<Vec<Registration>>,
}

impl Looper {
    fn create() -> Option<&'static Self> {
        // SAFETY: both calls have no pointer arguments and create owned descriptors.
        let epoll = unsafe { epoll_create1(0) };
        // SAFETY: eventfd returns a new descriptor owned by this looper.
        let wake_fd = unsafe { eventfd(0, 0) };
        if epoll < 0 || wake_fd < 0 {
            if epoll >= 0 {
                // SAFETY: `epoll` was just created and is not shared.
                unsafe { close(epoll) };
            }
            if wake_fd >= 0 {
                // SAFETY: `wake_fd` was just created and is not shared.
                unsafe { close(wake_fd) };
            }
            return None;
        }
        let mut event = EpollEvent {
            events: EPOLLIN,
            data: wake_fd as u64,
        };
        // SAFETY: descriptors are live and `event` is writable for the call.
        if unsafe { epoll_ctl(epoll, EPOLL_CTL_ADD, wake_fd, &mut event) } < 0 {
            // SAFETY: these are the two owned descriptors created above.
            unsafe {
                close(epoll);
                close(wake_fd);
            }
            return None;
        }
        Some(Box::leak(Box::new(Self {
            epoll,
            wake_fd,
            refs: AtomicUsize::new(1),
            registrations: RefCell::new(Vec::new()),
        })))
    }
}

thread_local! {
    static CURRENT: RefCell<Option<&'static Looper>> = const { RefCell::new(None) };
}

fn for_thread() -> Option<&'static Looper> {
    CURRENT.with(|current| *current.borrow())
}

fn as_looper(pointer: *mut c_void) -> Option<&'static Looper> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: pointers returned by prepare/forThread point to leaked Loopers.
    Some(unsafe { &*(pointer.cast::<Looper>()) })
}

extern "C" fn prepare(_options: c_int) -> *mut c_void {
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        if current.is_none() {
            *current = Looper::create();
        }
        current.map_or(std::ptr::null_mut(), |looper| {
            (looper as *const Looper).cast_mut().cast()
        })
    })
}

/// Prepare Android's looper for the calling thread, as the framework does for
/// an Activity's UI thread before calling into native GameActivity code.
///
/// `ALooper_forThread` intentionally remains a lookup and returns null when a
/// thread has not been prepared. Embedders that call GameActivity from their
/// own UI thread must prepare it first; AGDK returns a null native handle if
/// no looper is associated with that thread.
pub fn prepare_for_current_thread() -> bool {
    !prepare(0).is_null()
}

/// Poll the Android looper associated with the calling thread once.
///
/// Desktop hosts must keep pumping this after GameActivity registers its
/// command and input descriptors. Returning the Android poll result lets the
/// embedding event loop decide whether to schedule another immediate pass.
pub fn poll_for_current_thread(timeout_ms: c_int) -> Option<c_int> {
    if for_thread().is_none() {
        return None;
    }
    Some(poll_once(
        timeout_ms,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    ))
}

extern "C" fn for_thread_c() -> *mut c_void {
    for_thread().map_or(std::ptr::null_mut(), |looper| {
        (looper as *const Looper).cast_mut().cast()
    })
}

extern "C" fn acquire(pointer: *mut c_void) {
    if let Some(looper) = as_looper(pointer) {
        looper.refs.fetch_add(1, Ordering::Relaxed);
    }
}

extern "C" fn release(pointer: *mut c_void) {
    if let Some(looper) = as_looper(pointer) {
        looper.refs.fetch_sub(1, Ordering::Relaxed);
    }
}

extern "C" fn add_fd(
    pointer: *mut c_void,
    fd: c_int,
    ident: c_int,
    events: c_int,
    callback: Option<Callback>,
    data: *mut c_void,
) -> c_int {
    let Some(looper) = as_looper(pointer) else {
        return 0;
    };
    let mut flags = 0;
    if events & EVENT_INPUT != 0 {
        flags |= EPOLLIN;
    }
    if events & EVENT_OUTPUT != 0 {
        flags |= EPOLLOUT;
    }
    let mut event = EpollEvent {
        events: flags,
        data: fd as u64,
    };
    // SAFETY: the caller owns `fd`; the event and looper epoll descriptor are live.
    if unsafe { epoll_ctl(looper.epoll, EPOLL_CTL_ADD, fd, &mut event) } < 0 {
        return 0;
    }
    looper.registrations.borrow_mut().push(Registration {
        fd,
        ident: if callback.is_some() {
            POLL_CALLBACK
        } else {
            ident
        },
        callback,
        data,
    });
    1
}

extern "C" fn remove_fd(pointer: *mut c_void, fd: c_int) -> c_int {
    let Some(looper) = as_looper(pointer) else {
        return 0;
    };
    // SAFETY: epoll ignores the event pointer for EPOLL_CTL_DEL.
    unsafe {
        epoll_ctl(looper.epoll, EPOLL_CTL_DEL, fd, std::ptr::null_mut());
    }
    let mut registrations = looper.registrations.borrow_mut();
    let before = registrations.len();
    registrations.retain(|registration| registration.fd != fd);
    i32::from(registrations.len() != before)
}

fn translated_events(events: u32) -> c_int {
    let mut result = 0;
    if events & EPOLLIN != 0 {
        result |= EVENT_INPUT;
    }
    if events & EPOLLOUT != 0 {
        result |= EVENT_OUTPUT;
    }
    if events & EPOLLERR != 0 {
        result |= EVENT_ERROR;
    }
    if events & EPOLLHUP != 0 {
        result |= EVENT_HANGUP;
    }
    result
}

extern "C" fn poll_once(
    timeout: c_int,
    out_fd: *mut c_int,
    out_events: *mut c_int,
    out_data: *mut *mut c_void,
) -> c_int {
    let Some(looper) = for_thread() else {
        return POLL_ERROR;
    };
    // A bounded wait lets the engine recover from a lost wake-up.
    let timeout = if timeout < 0 { 50 } else { timeout };
    let mut events = [EpollEvent { events: 0, data: 0 }; 16];
    // SAFETY: `events` has the advertised capacity and `looper.epoll` is live.
    let count = unsafe {
        epoll_wait(
            looper.epoll,
            events.as_mut_ptr(),
            events.len() as c_int,
            timeout,
        )
    };
    if count < 0 {
        return POLL_ERROR;
    }
    if count == 0 {
        return POLL_TIMEOUT;
    }
    for event in events.iter().take(count as usize) {
        let fd = event.data as RawFd;
        if fd == looper.wake_fd {
            let mut value = 0u64;
            // SAFETY: eventfd requires an eight-byte read to drain its counter.
            unsafe {
                read(fd, (&mut value as *mut u64).cast(), 8);
            }
            return POLL_WAKE;
        }
        let registration = {
            looper
                .registrations
                .borrow()
                .iter()
                .find(|r| r.fd == fd)
                .map(|r| (r.ident, r.callback, r.data))
        };
        let Some((ident, callback, data)) = registration else {
            continue;
        };
        let event_bits = translated_events(event.events);
        if let Some(callback) = callback {
            if callback(fd, event_bits, data) == 0 {
                remove_fd((looper as *const Looper).cast_mut().cast(), fd);
            }
            return POLL_CALLBACK;
        }
        // SAFETY: each output pointer is optional per the Android API.
        unsafe {
            if !out_fd.is_null() {
                *out_fd = fd;
            }
            if !out_events.is_null() {
                *out_events = event_bits;
            }
            if !out_data.is_null() {
                *out_data = data;
            }
        }
        return ident;
    }
    POLL_TIMEOUT
}

extern "C" fn wake(pointer: *mut c_void) {
    let Some(looper) = as_looper(pointer) else {
        return;
    };
    let value = 1u64;
    // SAFETY: eventfd requires an eight-byte write; wake_fd is owned by looper.
    unsafe {
        write(looper.wake_fd, (&value as *const u64).cast(), 8);
    }
}

/// Android symbols implemented by the epoll-backed looper.
pub fn overrides() -> Vec<(&'static str, *mut c_void)> {
    macro_rules! f {
        ($name:literal, $function:expr) => {
            ($name, $function as *const () as *mut c_void)
        };
    }
    vec![
        f!("ALooper_prepare", prepare),
        f!("ALooper_forThread", for_thread_c),
        f!("ALooper_acquire", acquire),
        f!("ALooper_release", release),
        f!("ALooper_addFd", add_fd),
        f!("ALooper_removeFd", remove_fd),
        f!("ALooper_pollOnce", poll_once),
        f!("ALooper_wake", wake),
    ]
}
