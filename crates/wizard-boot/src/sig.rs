//! Signal flags for the fallback. The handlers only set an atomic; the
//! fallback's read loop looks at the flags, so the terminal is always put
//! back by the normal return path (a `Drop` guard), never from a handler.

// The only unsafe code of the program: installing three handlers.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPT: AtomicBool = AtomicBool::new(false);
static TERMINATE: AtomicBool = AtomicBool::new(false);
static HANGUP: AtomicBool = AtomicBool::new(false);

extern "C" fn handler(sig: libc::c_int) {
    // Only async-signal-safe work: an atomic store.
    let flag = match sig {
        libc::SIGINT => &INTERRUPT,
        libc::SIGTERM => &TERMINATE,
        _ => &HANGUP,
    };
    flag.store(true, Ordering::SeqCst);
}

/// Installs the handlers for SIGINT, SIGTERM and SIGHUP, without
/// `SA_RESTART` so a blocked read comes back.
pub fn install() {
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: `sa` is fully initialised (zeroed, then the handler and an
        // empty mask), the handler is an `extern "C"` function that only
        // does an atomic store, and the old action is not wanted (null).
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = handler as *const () as libc::sighandler_t;
            libc::sigemptyset(&mut sa.sa_mask);
            sa.sa_flags = 0;
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

/// Ctrl+C was pressed since the last call.
pub fn take_interrupt() -> bool {
    INTERRUPT.swap(false, Ordering::SeqCst)
}

/// SIGTERM arrived (systemd stopping the unit).
pub fn terminated() -> bool {
    TERMINATE.load(Ordering::SeqCst)
}

/// The terminal went away.
pub fn hung_up() -> bool {
    HANGUP.load(Ordering::SeqCst)
}
