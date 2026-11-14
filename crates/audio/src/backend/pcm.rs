//! Pure helpers shared by the host backends. No device, no session, nothing
//! audible: these run as ordinary unit tests anywhere.

use std::collections::VecDeque;
use std::ffi::c_void;

/// One caller-owned buffer waiting to be drained (the OpenSL buffer-queue
/// contract: valid and unmodified until its context is reported drained).
#[derive(Debug, Clone, Copy)]
pub struct PendingBuffer {
    pub data: *const u8,
    pub size: u32,
    pub offset: u32,
    pub context: *mut c_void,
}

/// Copies from the front of `pending` into `dst`, popping buffers as they
/// empty (their `context` goes to `drained`, in order) and zero-filling any
/// shortfall. Returns the number of trailing bytes padded with silence.
///
/// Silence, never stale bytes: reused stream buffers would otherwise replay
/// old audio on a loop, which is how a gap turns into an audible tone.
pub fn fill_pcm(
    pending: &mut VecDeque<PendingBuffer>,
    dst: &mut [u8],
    drained: &mut Vec<*mut c_void>,
) -> usize {
    let mut filled = 0usize;
    while filled < dst.len() {
        let Some(front) = pending.front_mut() else {
            break;
        };
        let avail = (front.size - front.offset) as usize;
        let take = avail.min(dst.len() - filled);
        // SAFETY: the enqueue contract guarantees `data` points to `size`
        // readable bytes until the buffer's context is reported drained, and
        // `offset + take <= size` by construction above.
        let src =
            unsafe { std::slice::from_raw_parts(front.data.add(front.offset as usize), take) };
        dst[filled..filled + take].copy_from_slice(src);
        front.offset += take as u32;
        filled += take;
        if front.offset >= front.size {
            drained.push(front.context);
            pending.pop_front();
        }
    }
    let padded = dst.len() - filled;
    dst[filled..].fill(0);
    padded
}

/// Which sink a stream should target. Empty `requested` means "follow the
/// default" and is not a fallback. A named sink that is absent falls back to
/// the default and reports `true` so the caller can warn. Matching is exact.
pub fn choose_output_target(requested: &str, available: &[String]) -> (String, bool) {
    if requested.is_empty() {
        return (String::new(), false);
    }
    if available.iter().any(|n| n == requested) {
        return (requested.to_owned(), false);
    }
    (String::new(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    fn pb(src: &[u8], ctx: usize) -> PendingBuffer {
        PendingBuffer {
            data: src.as_ptr(),
            size: src.len() as u32,
            offset: 0,
            context: ctx as *mut c_void,
        }
    }

    #[test]
    fn a_full_queue_fills_the_buffer_exactly() {
        let src = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut q = VecDeque::from([pb(&src, 0)]);
        let mut dst = [0xAAu8; 8];
        let mut drained = Vec::new();
        assert_eq!(fill_pcm(&mut q, &mut dst, &mut drained), 0);
        assert_eq!(dst, src);
        assert_eq!(drained.len(), 1);
        assert!(q.is_empty());
    }

    #[test]
    fn an_empty_queue_produces_silence_not_stale_bytes() {
        let mut q = VecDeque::new();
        let mut dst = [0xAAu8; 8];
        let mut drained = Vec::new();
        assert_eq!(fill_pcm(&mut q, &mut dst, &mut drained), 8);
        assert!(dst.iter().all(|&b| b == 0));
        assert!(drained.is_empty());
    }

    #[test]
    fn a_short_queue_pads_the_rest_with_silence() {
        let src = [9u8, 9, 9];
        let mut q = VecDeque::from([pb(&src, 0)]);
        let mut dst = [0xAAu8; 8];
        let mut drained = Vec::new();
        assert_eq!(fill_pcm(&mut q, &mut dst, &mut drained), 5);
        assert_eq!(&dst[..3], &src);
        assert!(dst[3..].iter().all(|&b| b == 0));
        assert_eq!(drained.len(), 1);
    }

    #[test]
    fn a_partially_consumed_buffer_is_not_dropped_early() {
        let src = [1u8, 2, 3, 4];
        let mut q = VecDeque::from([pb(&src, 0x1234)]);
        let mut dst = [0u8; 2];
        let mut drained = Vec::new();
        assert_eq!(fill_pcm(&mut q, &mut dst, &mut drained), 0);
        assert_eq!(dst, [1, 2]);
        assert!(drained.is_empty());
        assert_eq!(q.len(), 1);
        assert_eq!(q.front().unwrap().offset, 2);
    }

    #[test]
    fn multiple_buffers_are_drained_in_order_within_one_fill() {
        let (a, b) = ([1u8, 2], [3u8, 4]);
        let mut q = VecDeque::from([pb(&a, 1), pb(&b, 2)]);
        let mut dst = [0u8; 4];
        let mut drained = Vec::new();
        assert_eq!(fill_pcm(&mut q, &mut dst, &mut drained), 0);
        assert_eq!(dst, [1, 2, 3, 4]);
        assert_eq!(drained, vec![1usize as *mut c_void, 2usize as *mut c_void]);
        assert!(q.is_empty());
        let _ = ptr::null::<u8>();
    }

    fn sinks(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_chosen_sink_that_is_present_is_the_target() {
        let s = sinks(&["alsa_output.x", "bluez_output.AC_12"]);
        assert_eq!(
            choose_output_target("bluez_output.AC_12", &s),
            ("bluez_output.AC_12".into(), false)
        );
    }

    #[test]
    fn a_chosen_sink_that_has_gone_falls_back_and_says_so() {
        assert_eq!(
            choose_output_target("bluez_output.AC_12", &sinks(&["alsa_output.x"])),
            (String::new(), true)
        );
    }

    #[test]
    fn no_choice_follows_the_default_and_is_not_a_fallback() {
        assert_eq!(
            choose_output_target("", &sinks(&["alsa_output.x"])),
            (String::new(), false)
        );
    }

    #[test]
    fn an_empty_session_still_falls_back() {
        assert_eq!(
            choose_output_target("alsa_output.anything", &[]),
            (String::new(), true)
        );
    }

    #[test]
    fn matching_is_exact_rather_than_by_prefix_or_description() {
        let s = sinks(&["card.HiFi__HDMI1__sink", "card.HiFi__HDMI2__sink"]);
        assert_eq!(choose_output_target("card.HiFi__HDMI2__sink", &s).1, false);
        assert_eq!(
            choose_output_target("Built-in Audio (HDMI 2)", &s),
            (String::new(), true)
        );
    }
}
