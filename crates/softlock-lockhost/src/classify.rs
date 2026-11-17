//! Physical versus injected input, plus the test-only simulated-physical class.
//!
//! On Windows an event is software input when the low-level hook sets
//! `LLKHF_INJECTED` or `LLMHF_INJECTED`. The test feature additionally treats
//! an injected event as simulated-physical when `dwExtraInfo` equals the
//! per-run nonce. Release builds compile this module without that branch.

#[cfg(feature = "simulated-physical")]
use std::sync::atomic::{AtomicU64, Ordering};

/// One low-level keyboard or mouse event, reduced to the fields classification needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookInput {
    /// The hook reported `LLKHF_INJECTED` or `LLMHF_INJECTED`.
    pub injected: bool,
    /// `dwExtraInfo` from the hook structure.
    pub extra_info: u64,
}

/// How the lock host treats one input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputClass {
    /// The hook did not mark the event injected.
    Physical,
    /// Software input. It passes through the guard.
    Injected,
    /// Test-only. Injected, and tagged with this run's nonce.
    #[cfg(feature = "simulated-physical")]
    SimulatedPhysical,
}

/// Classifies one hook event. Without the test feature, injected events stay injected.
pub fn classify(input: HookInput) -> InputClass {
    #[cfg(feature = "simulated-physical")]
    {
        let _ = crate::marker::marker_bytes();
        let nonce = RUN_NONCE.load(Ordering::SeqCst);
        if input.injected && nonce != 0 && input.extra_info == nonce {
            return InputClass::SimulatedPhysical;
        }
    }
    if input.injected {
        InputClass::Injected
    } else {
        InputClass::Physical
    }
}

/// Per-run nonce written into `dwExtraInfo` by seam tests.
#[cfg(feature = "simulated-physical")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunNonce(u64);

#[cfg(feature = "simulated-physical")]
impl RunNonce {
    /// Draws a non-zero nonce. Zero means "no test nonce" inside [`classify`].
    pub fn random() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 8];
        getrandom::getrandom(&mut bytes)?;
        let value = u64::from_le_bytes(bytes) | 1;
        Ok(Self(value))
    }

    /// Test helper. Production callers use [`Self::random`] so the nonce comes from the CSPRNG.
    #[cfg(test)]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_extra_info(self) -> u64 {
        self.0
    }
}

#[cfg(feature = "simulated-physical")]
static RUN_NONCE: AtomicU64 = AtomicU64::new(0);

/// The nonce installed for this process, or zero when none is installed.
#[cfg(feature = "simulated-physical")]
pub fn current_run_nonce() -> u64 {
    RUN_NONCE.load(Ordering::SeqCst)
}

/// Installs the nonce for this process. Pass [`RunNonce::random`] once per test run.
#[cfg(feature = "simulated-physical")]
pub fn set_run_nonce(nonce: RunNonce) {
    RUN_NONCE.store(nonce.as_extra_info(), Ordering::SeqCst);
}

/// Forgets the nonce. Later injected events are classified as injected.
#[cfg(feature = "simulated-physical")]
pub fn clear_run_nonce() {
    RUN_NONCE.store(0, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::{HookInput, InputClass, classify};

    #[cfg(not(feature = "simulated-physical"))]
    #[test]
    fn release_classifier_keeps_a_nonce_looking_event_injected() {
        let tagged = classify(HookInput {
            injected: true,
            extra_info: 0xA5A5_5A5A_1234_5678,
        });
        assert_eq!(tagged, InputClass::Injected);
        let hardware = classify(HookInput {
            injected: false,
            extra_info: 0xA5A5_5A5A_1234_5678,
        });
        assert_eq!(hardware, InputClass::Physical);
    }

    #[cfg(feature = "simulated-physical")]
    #[test]
    fn nonce_tagged_injected_event_is_simulated_physical() {
        use super::{RunNonce, clear_run_nonce, set_run_nonce};

        let nonce = RunNonce::random().expect("nonce");
        assert_ne!(nonce.as_extra_info(), 0);
        set_run_nonce(nonce);

        let tagged = classify(HookInput {
            injected: true,
            extra_info: nonce.as_extra_info(),
        });
        assert_eq!(tagged, InputClass::SimulatedPhysical);

        let plain = classify(HookInput {
            injected: true,
            extra_info: 0,
        });
        assert_eq!(plain, InputClass::Injected);

        let other = classify(HookInput {
            injected: true,
            extra_info: nonce.as_extra_info() ^ 0x00FF_0000_0000_0000,
        });
        assert_eq!(other, InputClass::Injected);

        let hardware = classify(HookInput {
            injected: false,
            extra_info: nonce.as_extra_info(),
        });
        assert_eq!(hardware, InputClass::Physical);

        clear_run_nonce();
        let after_clear = classify(HookInput {
            injected: true,
            extra_info: nonce.as_extra_info(),
        });
        assert_eq!(after_clear, InputClass::Injected);

        let fixed = RunNonce::from_raw(0x1111_2222_3333_4444);
        set_run_nonce(fixed);
        assert_eq!(
            classify(HookInput {
                injected: true,
                extra_info: fixed.as_extra_info(),
            }),
            InputClass::SimulatedPhysical
        );
        clear_run_nonce();
        let _ = crate::marker::marker_bytes();
    }
}
