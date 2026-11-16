//! Fixed ring of hook events. One producer (the hook thread), one consumer
//! (the window thread). The callback only stores a copy into a slot that
//! already exists.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) const CAP: usize = 256;
pub(crate) const KIND_KEY: u8 = 1;
pub(crate) const KIND_CLICK: u8 = 2;
pub(crate) const KIND_WHEEL: u8 = 3;

#[derive(Clone, Copy, Default)]
pub(crate) struct RawEvent {
    pub kind: u8,
    pub vk: u16,
    pub scan: u16,
}

pub(crate) struct EventQueue {
    slots: Box<[UnsafeCell<RawEvent>]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

// The hook thread writes a slot, then releases `tail`. The window thread
// acquires `tail`, then reads that slot. Each side owns one index.
unsafe impl Send for EventQueue {}
unsafe impl Sync for EventQueue {}

impl EventQueue {
    pub(crate) fn new() -> Self {
        let slots = (0..CAP)
            .map(|_| UnsafeCell::new(RawEvent::default()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            slots,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub(crate) fn push(&self, event: RawEvent) {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= CAP {
            return;
        }
        let index = tail % CAP;
        unsafe {
            *self.slots[index].get() = event;
        }
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
    }

    pub(crate) fn pop(&self) -> Option<RawEvent> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        let index = head % CAP;
        let event = unsafe { *self.slots[index].get() };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(event)
    }
}
