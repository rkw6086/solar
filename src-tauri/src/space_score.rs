//! The accepted Solar welcome theme, bundled for offline playback.
//!
//! This is Take 1 with softer percussion and pitch-preserving 8% slower
//! playback. The pre-mastered PCM asset has quiet edges for looping; native
//! volume and tour transitions are controlled separately.

#[cfg(any(target_os = "macos", test))]
const SCORE_WAV: &[u8] = include_bytes!("../assets/audio/solar-welcome.wav");

#[cfg(any(target_os = "macos", test))]
fn gain(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value.clamp(0.0, 1.0))
    } else {
        0.0
    }
}

/// Begin or continue the same loop. Call on the AppKit main thread.
///
/// For a gentle entrance, call `play(0.0)` followed by a short series of
/// `set_volume` calls. Changes adjust playback without restarting the timeline.
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub fn play(requested_volume: f32) -> Result<(), String> {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::NSSound;
    use objc2_foundation::NSData;

    MainThreadMarker::new().ok_or("The welcome score must start on the main thread")?;
    CURRENT_SCORE.with(|current| {
        let mut current = current.borrow_mut();
        if let Some(sound) = current.as_ref() {
            sound.setVolume(gain(requested_volume) as f32);
            if !sound.isPlaying() && !sound.play() {
                return Err("macOS could not resume the welcome score".into());
            }
            return Ok(());
        }
        let data = NSData::with_bytes(SCORE_WAV);
        let sound = NSSound::initWithData(NSSound::alloc(), &data)
            .ok_or("macOS could not decode the welcome score")?;
        sound.setLoops(true);
        sound.setVolume(gain(requested_volume) as f32);
        if !sound.play() {
            return Err("macOS could not play the welcome score".into());
        }
        *current = Some(sound);
        Ok(())
    })
}

/// Adjust a playing loop without creating another sound or restarting it.
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub fn set_volume(requested_volume: f32) -> Result<(), String> {
    objc2::MainThreadMarker::new().ok_or("The welcome score must update on the main thread")?;
    CURRENT_SCORE.with(|current| {
        if let Some(sound) = current.borrow().as_ref() {
            sound.setVolume(gain(requested_volume) as f32);
        }
    });
    Ok(())
}

/// Release the current loop. A caller can fade with `set_volume` before this.
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub fn stop() -> Result<(), String> {
    objc2::MainThreadMarker::new().ok_or("The welcome score must stop on the main thread")?;
    CURRENT_SCORE.with(|current| {
        if let Some(sound) = current.borrow_mut().take() {
            sound.stop();
        }
    });
    Ok(())
}

#[cfg(all(feature = "desktop", target_os = "macos"))]
thread_local! {
    static CURRENT_SCORE: std::cell::RefCell<Option<objc2::rc::Retained<objc2_app_kit::NSSound>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(all(feature = "desktop", not(target_os = "macos")))]
pub fn play(_requested_volume: f32) -> Result<(), String> {
    Err("Native welcome music playback is available on macOS".into())
}

#[cfg(all(feature = "desktop", not(target_os = "macos")))]
pub fn set_volume(_requested_volume: f32) -> Result<(), String> {
    Ok(())
}

#[cfg(all(feature = "desktop", not(target_os = "macos")))]
pub fn stop() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<i16> {
        SCORE_WAV[44..]
            .chunks_exact(2)
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect()
    }

    #[test]
    fn bundled_theme_is_complete_stereo_pcm() {
        assert_eq!(&SCORE_WAV[..4], b"RIFF");
        assert_eq!(&SCORE_WAV[8..16], b"WAVEfmt ");
        assert_eq!(&SCORE_WAV[36..40], b"data");
        assert_eq!(u16::from_le_bytes(SCORE_WAV[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(SCORE_WAV[22..24].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(SCORE_WAV[24..28].try_into().unwrap()),
            44_100
        );
        assert_eq!(
            u16::from_le_bytes(SCORE_WAV[34..36].try_into().unwrap()),
            16
        );
        let data_bytes = u32::from_le_bytes(SCORE_WAV[40..44].try_into().unwrap()) as usize;
        assert_eq!(SCORE_WAV.len(), data_bytes + 44);
        assert_eq!(data_bytes % 4, 0);
        let duration = data_bytes as f64 / 4.0 / 44_100.0;
        assert!((32.5..32.7).contains(&duration));
        assert!(SCORE_WAV.len() < 6 * 1024 * 1024);
    }

    #[test]
    fn accepted_master_has_headroom_and_quiet_loop_edges() {
        let samples = samples();
        let peak = samples
            .iter()
            .map(|value| i32::from(*value).abs())
            .max()
            .unwrap();
        let rms = (samples
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt()
            / f64::from(i16::MAX);
        assert!(peak > 5_000 && peak < 16_384);
        assert!((0.04..0.09).contains(&rms), "unexpected master RMS {rms}");
        assert!(
            samples[..512]
                .iter()
                .all(|value| value.unsigned_abs() < 300)
        );
        assert!(
            samples[samples.len() - 512..]
                .iter()
                .all(|value| value.unsigned_abs() < 300)
        );
        for channel in 0..2 {
            let seam =
                i32::from(samples[channel]) - i32::from(samples[samples.len() - 2 + channel]);
            assert!(seam.abs() < 40, "loop boundary jump {seam}");
        }
    }

    #[test]
    fn playback_gain_is_finite_and_bounded() {
        assert_eq!(gain(0.12), f64::from(0.12_f32));
        assert_eq!(gain(2.0), 1.0);
        for value in [0.0, -0.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(gain(value), 0.0);
        }
    }
}
