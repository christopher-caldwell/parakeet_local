//! Scoped C callbacks. The context outlives the synchronous whisper full call.
use parakeet_application::processing_control::*;
use std::ffi::c_void;
pub struct Callbacks {
    pub control: ProcessingControl,
    pub consumed: i64,
    pub duration: i64,
    pub chunks: usize,
}
unsafe extern "C" fn abort(data: *mut c_void) -> bool {
    // SAFETY: install points at the caller-owned box, held through full().
    unsafe { (&*data.cast::<Callbacks>()).control.is_cancelled() }
}
unsafe extern "C" fn progress(
    _: *mut whisper_rs::whisper_rs_sys::whisper_context,
    _: *mut whisper_rs::whisper_rs_sys::whisper_state,
    percent: i32,
    data: *mut c_void,
) {
    // SAFETY: identical lifetime to abort; state access is synchronized.
    let c = unsafe { &*data.cast::<Callbacks>() };
    c.control.publish(
        ProcessingStage::Transcribing,
        c.consumed,
        Some(c.duration),
        c.chunks,
        Some(percent.clamp(0, 100) as u8),
    );
}
impl Callbacks {
    pub fn run(
        &self,
        state: &mut whisper_rs::WhisperState,
        mut params: whisper_rs::FullParams<'_, '_>,
        samples: &[f32],
    ) -> Result<(), whisper_rs::WhisperError> {
        let data = (self as *const Self).cast_mut().cast::<c_void>();
        // SAFETY: SDK callbacks borrow this box only during the synchronous full
        // invocation. No closure is transferred to or allocated by the SDK.
        unsafe {
            params.set_abort_callback(Some(abort));
            params.set_abort_callback_user_data(data);
            params.set_progress_callback(Some(progress));
            params.set_progress_callback_user_data(data);
        }
        state.full(params, samples)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_callbacks_report_progress_and_read_cancellation() {
        let c = Box::new(Callbacks {
            control: ProcessingControl::default(),
            consumed: 60000,
            duration: 120000,
            chunks: 1,
        });
        let p = (&*c as *const Callbacks).cast_mut().cast();
        unsafe {
            progress(std::ptr::null_mut(), std::ptr::null_mut(), 42, p);
            assert!(!abort(p));
        }
        assert_eq!(c.control.snapshot().current_chunk_percent, Some(42));
        c.control.request_cancellation();
        unsafe {
            assert!(abort(p));
        }
    }
}
