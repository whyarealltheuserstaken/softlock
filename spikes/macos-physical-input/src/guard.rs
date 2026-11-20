//! Releases the listen-only tap on drop, including panic unwind.

pub struct TapGuard<F>
where
    F: FnOnce(),
{
    release: Option<F>,
}

impl<F> TapGuard<F>
where
    F: FnOnce(),
{
    pub fn new(release: F) -> Self {
        Self {
            release: Some(release),
        }
    }
}

impl<F> Drop for TapGuard<F>
where
    F: FnOnce(),
{
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TapGuard;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn dropping_the_guard_releases_the_tap_even_when_dispatch_unwinds() {
        static RELEASED: AtomicBool = AtomicBool::new(false);
        RELEASED.store(false, Ordering::SeqCst);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = TapGuard::new(|| RELEASED.store(true, Ordering::SeqCst));
            panic!("probe panicked");
        }));
        assert!(result.is_err());
        assert!(
            RELEASED.load(Ordering::SeqCst),
            "TapGuard Drop must release the tap on unwind"
        );
    }
}
