//! Short, locally synthesized feedback for Solar's introduction and controls.
//!
//! All cues include their release tail. The native player keeps a small pool
//! so ripples can overlap without leaving sounds alive after mute or dismissal.

const SAMPLE_RATE: u32 = 44_100;
#[cfg(any(test, all(feature = "desktop", target_os = "macos")))]
const MAX_VOICES: usize = 8;

fn gain(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value.clamp(0.0, 1.0))
    } else {
        0.0
    }
}

fn smooth(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn duration_ms(cue: &str) -> Result<u32, String> {
    match cue {
        "click" => Ok(32),
        "ripple" => Ok(190),
        "formation" => Ok(900),
        "wink" => Ok(800),
        _ => Err("Unknown Solar sound effect".into()),
    }
}

fn wave(samples: impl IntoIterator<Item = f64>, count: usize, volume: f64) -> Vec<u8> {
    let data_length = (count * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_length as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_length).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_length.to_le_bytes());
    for sample in samples {
        let sample = (sample * volume).clamp(-1.0, 1.0);
        wav.extend_from_slice(&((sample * f64::from(i16::MAX)) as i16).to_le_bytes());
    }
    wav
}

/// Build a mono PCM WAV for a cue. Variants change the voice, never its length.
/// The caller chooses variants; playback does not require random or network state.
pub fn synthesize(cue: &str, variant: u32, volume: f32) -> Result<Vec<u8>, String> {
    let duration = duration_ms(cue)?;
    if cue == "wink" {
        // The sun uses the same signature language as its Orbit companions.
        return Ok(crate::attention_sound::synthesize_chime(
            &format!("solar-wink-{}", variant % 64),
            (gain(volume) * 0.62) as f32,
        ));
    }
    let count = (SAMPLE_RATE * duration / 1_000) as usize;
    let length = f64::from(duration) / 1_000.0;
    let mut filtered_noise = 0.0;
    let samples = (0..count).map(|offset| {
        let time = offset as f64 / f64::from(SAMPLE_RATE);
        let remaining = length - time;
        match cue {
            "click" => {
                // A tiny dry mechanical tick, with the bright edge rounded off.
                let bits = (offset as u32)
                    .wrapping_mul(747_796_405)
                    .wrapping_add(2_891_336_453);
                let noise = f64::from((bits ^ (bits >> 16)) & 0xffff) / 32767.5 - 1.0;
                filtered_noise += 0.32 * (noise - filtered_noise);
                let envelope =
                    smooth(time / 0.0015) * (-time * 190.0).exp() * smooth(remaining / 0.008);
                let body = (std::f64::consts::TAU * 780.0 * time).sin();
                0.22 * envelope * (0.62 * filtered_noise + 0.38 * body)
            }
            "ripple" => {
                // A rounded water/glass drop; the brief pitch settling makes
                // the impact feel physical without a sharp notification edge.
                let notes = [60, 62, 64, 67, 69, 72];
                let midi = notes[variant as usize % notes.len()];
                let frequency = 440.0 * 2.0_f64.powf((f64::from(midi) - 69.0) / 12.0);
                let phase = std::f64::consts::TAU
                    * frequency
                    * (time + 0.0025 * (1.0 - (-time * 42.0).exp()));
                let envelope =
                    smooth(time / 0.005) * (-time * 22.0).exp() * smooth(remaining / 0.05);
                0.24 * envelope
                    * (0.88 * phase.sin() + 0.12 * (-time * 40.0).exp() * (phase * 2.65).sin())
            }
            "formation" => {
                // Open C major/add9, kept airy with slow attacks and only a
                // little octave color. This resolves before the visual zoom.
                let frequencies = [261.625_565, 329.627_557, 391.995_436, 587.329_536];
                let envelope = smooth(time / 0.10) * smooth(remaining / 0.22);
                let shimmer = 0.96 + 0.04 * (std::f64::consts::TAU * 1.2 * time).sin();
                let chord = frequencies
                    .into_iter()
                    .enumerate()
                    .map(|(index, frequency)| {
                        let phase = std::f64::consts::TAU * frequency * time;
                        let detune = if index % 2 == 0 { 1.0015 } else { 0.9985 };
                        0.80 * phase.sin()
                            + 0.17 * (phase * detune).sin()
                            + 0.03 * (phase * 2.0).sin()
                    })
                    .sum::<f64>();
                0.055 * envelope * shimmer * chord
            }
            _ => unreachable!("cue already validated"),
        }
    });
    Ok(wave(samples, count, gain(volume)))
}

/// Play on the macOS main thread, allowing a bounded number of overlapping cues.
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub fn play(cue: &str, variant: u32, volume: f32) -> Result<(), String> {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::NSSound;
    use objc2_foundation::NSData;
    use std::time::{Duration, Instant};

    MainThreadMarker::new().ok_or("Sound effects must play on the main thread")?;
    duration_ms(cue)?;
    if gain(volume) == 0.0 {
        return Ok(());
    }
    SOUNDS.with(|sounds| {
        let mut sounds = sounds.borrow_mut();
        let now = Instant::now();
        if cue == "click" {
            if sounds
                .last_click
                .is_some_and(|last| now.duration_since(last) < Duration::from_millis(45))
            {
                return Ok(());
            }
            sounds.last_click = Some(now);
        }
        sounds.voices.retain(|voice| voice.sound.isPlaying());
        if matches!(cue, "formation" | "wink") {
            sounds.voices.retain(|voice| {
                if voice.cue == cue {
                    voice.sound.stop();
                    false
                } else {
                    true
                }
            });
        }
        while sounds.voices.len() >= MAX_VOICES {
            sounds.voices.remove(0).sound.stop();
        }
        let data = NSData::with_bytes(&synthesize(cue, variant, volume)?);
        let sound = NSSound::initWithData(NSSound::alloc(), &data)
            .ok_or("macOS could not decode the sound effect")?;
        sound.setLoops(false);
        sound.setVolume(1.0);
        if !sound.play() {
            return Err("macOS could not play the sound effect".into());
        }
        sounds.voices.push(NativeVoice {
            cue: cue.into(),
            sound,
        });
        Ok(())
    })
}

/// Stop every interaction cue, including a formation tone that is mid-release.
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub fn stop() -> Result<(), String> {
    objc2::MainThreadMarker::new().ok_or("Sound effects must stop on the main thread")?;
    SOUNDS.with(|sounds| {
        let mut sounds = sounds.borrow_mut();
        for voice in sounds.voices.drain(..) {
            voice.sound.stop();
        }
        sounds.last_click = None;
    });
    Ok(())
}

#[cfg(all(feature = "desktop", target_os = "macos"))]
struct NativeVoice {
    cue: String,
    sound: objc2::rc::Retained<objc2_app_kit::NSSound>,
}

#[cfg(all(feature = "desktop", target_os = "macos"))]
#[derive(Default)]
struct NativeSounds {
    voices: Vec<NativeVoice>,
    last_click: Option<std::time::Instant>,
}

#[cfg(all(feature = "desktop", target_os = "macos"))]
thread_local! {
    static SOUNDS: std::cell::RefCell<NativeSounds> =
        std::cell::RefCell::new(NativeSounds::default());
}

#[cfg(all(feature = "desktop", not(target_os = "macos")))]
pub fn play(cue: &str, variant: u32, volume: f32) -> Result<(), String> {
    duration_ms(cue)?;
    if gain(volume) == 0.0 {
        return Ok(());
    }
    let _ = synthesize(cue, variant, volume)?;
    Err("Native sound effects are available on macOS".into())
}

#[cfg(all(feature = "desktop", not(target_os = "macos")))]
pub fn stop() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(wav: &[u8]) -> Vec<i16> {
        wav[44..]
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
            .collect()
    }

    #[test]
    fn effects_have_valid_bounded_pcm_with_complete_release_tails() {
        for (cue, milliseconds) in [
            ("click", 32),
            ("ripple", 190),
            ("formation", 900),
            ("wink", 800),
        ] {
            let wav = synthesize(cue, 3, 1.0).unwrap();
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            assert_eq!(&wav[36..40], b"data");
            assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
            assert_eq!(
                u32::from_le_bytes(wav[24..28].try_into().unwrap()),
                SAMPLE_RATE
            );
            let samples = samples(&wav);
            assert_eq!(samples.len(), (SAMPLE_RATE * milliseconds / 1_000) as usize);
            assert!(samples.iter().any(|sample| *sample != 0));
            assert_eq!(samples[0], 0);
            assert!(
                samples[samples.len() - 8..]
                    .iter()
                    .all(|sample| sample.unsigned_abs() <= 2)
            );
            assert!(wav.len() < 100_000);
        }
    }

    #[test]
    fn effects_are_quiet_deterministic_and_volume_bounded() {
        for cue in ["click", "ripple", "formation", "wink"] {
            let full = synthesize(cue, 7, 1.0).unwrap();
            assert_eq!(full, synthesize(cue, 7, 2.0).unwrap());
            let peak = samples(&full)
                .into_iter()
                .map(i16::unsigned_abs)
                .max()
                .unwrap();
            assert!((100..12_000).contains(&peak), "{cue} peak {peak}");
            let quiet = samples(&synthesize(cue, 7, 0.25).unwrap());
            for (a, b) in samples(&full).iter().zip(quiet) {
                assert!((i32::from(*a) / 4 - i32::from(b)).abs() <= 2);
            }
            for volume in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert!(
                    samples(&synthesize(cue, 7, volume).unwrap())
                        .iter()
                        .all(|sample| *sample == 0)
                );
            }
        }
        assert_eq!(MAX_VOICES, 8);
        assert!(synthesize("music", 0, 1.0).is_err());
    }

    #[test]
    fn ripple_variants_and_sun_signatures_change_without_changing_length() {
        for cue in ["ripple", "wink"] {
            let first = synthesize(cue, 0, 1.0).unwrap();
            let second = synthesize(cue, 1, 1.0).unwrap();
            assert_ne!(first, second);
            assert_eq!(first.len(), second.len());
        }
        let expected = crate::attention_sound::synthesize_chime("solar-wink-9", 0.62);
        assert_eq!(synthesize("wink", 9, 1.0).unwrap(), expected);
    }
}
