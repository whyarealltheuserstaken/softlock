//! The PIN in a parsed JSON value has to be wiped on the paths that fail
//! before `Secret`'s own `Drop` runs. A watcher records heap blocks allocated
//! during the call and checks they do not still hold the PIN when freed.

use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use softlock_core::{IpcError, encode_frame, message_json};

const PIN: &str = "softlock-pin-canary-7f3c9a1e4b28d605";
const RING: usize = 1024;

struct WatchAlloc;

static WATCH: Mutex<()> = Mutex::new(());
static ARMED: AtomicBool = AtomicBool::new(false);
static PIN_PTR: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
static PIN_LEN: AtomicUsize = AtomicUsize::new(0);
static SLOT_COUNT: AtomicUsize = AtomicUsize::new(0);
static OVERFLOW: AtomicBool = AtomicBool::new(false);
static LEAKED: AtomicBool = AtomicBool::new(false);
static LEAK_LEN: AtomicUsize = AtomicUsize::new(0);
static SCANNED: AtomicUsize = AtomicUsize::new(0);
static SLOT_PTR: [AtomicPtr<u8>; RING] = [const { AtomicPtr::new(ptr::null_mut()) }; RING];
static SLOT_LEN: [AtomicUsize; RING] = [const { AtomicUsize::new(0) }; RING];

unsafe impl GlobalAlloc for WatchAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            note_alloc(ptr, layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_dealloc(ptr, layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: WatchAlloc = WatchAlloc;

fn note_alloc(ptr: *mut u8, size: usize) {
    if size == 0 || !ARMED.load(Ordering::Acquire) {
        return;
    }
    let index = SLOT_COUNT.fetch_add(1, Ordering::Relaxed);
    if index >= RING {
        OVERFLOW.store(true, Ordering::Relaxed);
        return;
    }
    // The block is initialized here so the free-time scan does not read
    // uninitialized capacity. Only blocks from this window are scanned.
    unsafe { ptr::write_bytes(ptr, 0xA5, size) };
    SLOT_LEN[index].store(size, Ordering::Relaxed);
    SLOT_PTR[index].store(ptr, Ordering::Release);
}

fn note_dealloc(ptr: *mut u8, size: usize) {
    if !ARMED.load(Ordering::Acquire) {
        return;
    }
    let Some(recorded) = take_slot(ptr) else {
        return;
    };
    if recorded != size || size == 0 {
        return;
    }
    let pin_ptr = PIN_PTR.load(Ordering::Acquire);
    let pin_len = PIN_LEN.load(Ordering::Acquire);
    if pin_ptr.is_null() || pin_len == 0 {
        return;
    }
    let pin = unsafe { std::slice::from_raw_parts(pin_ptr, pin_len) };
    let mem = unsafe { std::slice::from_raw_parts(ptr, size) };
    SCANNED.fetch_add(1, Ordering::Relaxed);
    if contains_bytes(mem, pin) {
        LEAKED.store(true, Ordering::Relaxed);
        LEAK_LEN.store(size, Ordering::Relaxed);
    }
}

fn take_slot(ptr: *mut u8) -> Option<usize> {
    let count = SLOT_COUNT.load(Ordering::Acquire).min(RING);
    for index in 0..count {
        if SLOT_PTR[index].load(Ordering::Acquire) == ptr {
            SLOT_PTR[index].store(ptr::null_mut(), Ordering::Relaxed);
            return Some(SLOT_LEN[index].load(Ordering::Relaxed));
        }
    }
    None
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn assert_pin_is_wiped(pin: &str, call: impl FnOnce()) {
    let _guard = WATCH.lock().unwrap_or_else(|poison| poison.into_inner());
    SLOT_COUNT.store(0, Ordering::Relaxed);
    OVERFLOW.store(false, Ordering::Relaxed);
    LEAKED.store(false, Ordering::Relaxed);
    LEAK_LEN.store(0, Ordering::Relaxed);
    SCANNED.store(0, Ordering::Relaxed);
    for index in 0..RING {
        SLOT_PTR[index].store(ptr::null_mut(), Ordering::Relaxed);
        SLOT_LEN[index].store(0, Ordering::Relaxed);
    }
    PIN_PTR.store(pin.as_ptr().cast_mut(), Ordering::Relaxed);
    PIN_LEN.store(pin.len(), Ordering::Relaxed);
    ARMED.store(true, Ordering::Release);
    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            ARMED.store(false, Ordering::Release);
        }
    }
    let _disarm = Disarm;
    call();
    assert!(
        !OVERFLOW.load(Ordering::Relaxed),
        "the allocation watch overflowed"
    );
    assert!(
        SCANNED.load(Ordering::Relaxed) > 0,
        "the call freed nothing the watch could see"
    );
    assert!(
        !LEAKED.load(Ordering::Acquire),
        "a freed block of {} bytes still held the PIN",
        LEAK_LEN.load(Ordering::Relaxed)
    );
}

#[test]
fn decode_request_zeroizes_a_pin_when_from_value_fails() {
    // Unknown kind fails inside `from_value` before the PIN string is moved
    // into `Secret`, so the parsed object still holds it.
    let json = format!(r#"{{"v":1,"id":7,"kind":"NotARequest","pin":"{PIN}"}}"#);
    let frame = encode_frame(&json).expect("frame");
    assert_pin_is_wiped(PIN, || {
        assert_eq!(
            softlock_core::decode_request_frame(&frame),
            Err(IpcError::Malformed)
        );
    });
}

#[test]
fn decode_request_zeroizes_a_pin_when_the_request_parses() {
    let json = format!(r#"{{"v":1,"id":7,"kind":"SetPin","pin":"{PIN}"}}"#);
    let frame = encode_frame(&json).expect("frame");
    assert_pin_is_wiped(PIN, || {
        let decoded = softlock_core::decode_request_frame(&frame).expect("set pin");
        assert!(matches!(
            decoded.kind,
            softlock_core::RequestKind::SetPin { .. }
        ));
    });
}

#[test]
fn message_json_zeroizes_the_original_non_object_value() {
    let body = serde_json::Value::String(PIN.to_string());
    assert_pin_is_wiped(PIN, || {
        assert_eq!(message_json(1, &body), Err(IpcError::Malformed));
    });
}
