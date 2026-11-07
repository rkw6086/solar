//! Short, deterministic Orbit signatures synthesized without sound assets.
//!
//! A MIDI-like pentatonic melody drives a small additive synthesizer. The WAV
//! always includes its release tail and is exactly 800 ms long.

const SAMPLE_RATE: u32 = 44_100;
const DURATION_MS: u32 = 800;
const NOTE_COUNT: usize = 5;
const NOTE_SPACING: f64 = 0.125;
const NOTE_DURATION: f64 = 0.275;

fn fingerprint(orbit_id: &str) -> u64 {
    // Unlike DefaultHasher, this gives an Orbit the same voice across launches
    // and Rust versions. IDs, rather than editable display names, own the voice.
    orbit_id.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn volume(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value.clamp(0.0, 1.0))
    } else {
        0.0
    }
}

/// Produce a little-endian, mono, 16-bit PCM WAV, including its RIFF header.
/// Every input produces an 800 ms buffer; volume never changes its duration.
pub fn synthesize_chime(orbit_id: &str, requested_volume: f32) -> Vec<u8> {
    let sample_count = (SAMPLE_RATE * DURATION_MS / 1000) as usize;
    let mut samples = vec![0.0_f64; sample_count];
    let identity = fingerprint(orbit_id);
    let gain = volume(requested_volume);
    let pentatonic = [0, 2, 4, 7, 9, 12, 14, 16, 19, 21];
    let root = 60 + ((identity >> 32) % 5) as i32;
    let harmonic_mix = 0.08 + ((identity >> 40) % 5) as f64 * 0.035;

    for note in 0..NOTE_COUNT {
        let degree = ((identity >> (note * 6)) % pentatonic.len() as u64) as usize;
        let midi = root + pentatonic[degree];
        let frequency = 440.0 * 2.0_f64.powf((f64::from(midi) - 69.0) / 12.0);
        let start = (note as f64 * NOTE_SPACING * f64::from(SAMPLE_RATE)) as usize;
        let length = (NOTE_DURATION * f64::from(SAMPLE_RATE)) as usize;
        for (offset, sample) in samples.iter_mut().skip(start).take(length).enumerate() {
            let time = offset as f64 / f64::from(SAMPLE_RATE);
            let attack = (time / 0.008).min(1.0);
            let release = ((NOTE_DURATION - time) / 0.065).clamp(0.0, 1.0);
            let envelope = attack * release * (-time * 8.0).exp();
            let phase = std::f64::consts::TAU * frequency * time;
            let voice = (1.0 - harmonic_mix) * phase.sin() + harmonic_mix * (phase * 2.0).sin();
            *sample += voice * envelope * 0.32 * gain;
        }
    }

    let data_length = (sample_count * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_length as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_length).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // Mono
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes()); // Block alignment
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_length.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&((sample.clamp(-1.0, 1.0) * i16::MAX as f64) as i16).to_le_bytes());
    }
    wav
}

/// Play on the macOS main thread. The previous chime is replaced, never layered.
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub fn play(orbit_id: &str, requested_volume: f32) -> Result<(), String> {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::NSSound;
    use objc2_foundation::NSData;

    MainThreadMarker::new().ok_or("Attention sound must start on the main thread")?;
    if volume(requested_volume) == 0.0 {
        return Ok(());
    }
    let data = NSData::with_bytes(&synthesize_chime(orbit_id, requested_volume));
    let sound = NSSound::initWithData(NSSound::alloc(), &data)
        .ok_or("macOS could not decode the attention chime")?;
    sound.setLoops(false);
    sound.setVolume(1.0);
    CURRENT_SOUND.with(|current| {
        let mut current = current.borrow_mut();
        if let Some(previous) = current.take() {
            previous.stop();
        }
        if !sound.play() {
            return Err("macOS could not play the attention chime".into());
        }
        // Retain only the most recent, <=800 ms sound. No delegate or delayed
        // task is needed, and ownership remains on the main thread.
        *current = Some(sound);
        Ok(())
    })
}

#[cfg(all(feature = "desktop", target_os = "macos"))]
thread_local! {
    static CURRENT_SOUND: std::cell::RefCell<Option<objc2::rc::Retained<objc2_app_kit::NSSound>>> =
        const { std::cell::RefCell::new(None) };
}

/// The synthesizer is portable; native output currently targets macOS.
#[cfg(all(feature = "desktop", not(target_os = "macos")))]
pub fn play(orbit_id: &str, requested_volume: f32) -> Result<(), String> {
    if volume(requested_volume) == 0.0 {
        return Ok(());
    }
    let _ = synthesize_chime(orbit_id, requested_volume);
    Err("Native attention sound playback is available on macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(wav: &[u8]) -> impl Iterator<Item = i16> + '_ {
        wav[44..]
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
    }

    #[test]
    fn valid_wav_is_exactly_eight_hundred_ms_including_release() {
        let wav = synthesize_chime("assistant", 1.0);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize,
            wav.len() - 8
        );
        assert_eq!(
            u32::from_le_bytes(wav[24..28].try_into().unwrap()),
            SAMPLE_RATE
        );
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
        assert_eq!(samples(&wav).count(), SAMPLE_RATE as usize * 8 / 10);
        assert!(samples(&wav).any(|sample| sample != 0));
        assert!(samples(&wav).take(1).all(|sample| sample == 0));
        assert!(
            samples(&wav)
                .skip(SAMPLE_RATE as usize * 79 / 100)
                .all(|sample| sample == 0)
        );
    }

    #[test]
    fn signature_is_stable_and_distinct_for_orbit_ids() {
        let first = synthesize_chime("assistant", 0.4);
        assert_eq!(first, synthesize_chime("assistant", 0.4));
        assert_ne!(first, synthesize_chime("researcher", 0.4));
        assert_ne!(first, synthesize_chime("qa", 0.4));
    }

    #[test]
    fn bounded_volume_and_envelopes_avoid_clipping_and_clicks() {
        let full = synthesize_chime("assistant", 1.0);
        let quiet = synthesize_chime("assistant", 0.25);
        let peak = |wav: &[u8]| {
            samples(wav)
                .map(|sample| i32::from(sample).abs())
                .max()
                .unwrap()
        };
        assert!(peak(&full) < i32::from(i16::MAX));
        assert!((peak(&quiet) - peak(&full) / 4).abs() <= 1);
        assert_eq!(full, synthesize_chime("assistant", 2.0));
        for requested in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(samples(&synthesize_chime("assistant", requested)).all(|sample| sample == 0));
        }
        let maximum_step = samples(&full)
            .collect::<Vec<_>>()
            .windows(2)
            .map(|pair| (i32::from(pair[1]) - i32::from(pair[0])).abs())
            .max()
            .unwrap();
        assert!(maximum_step < 4000);
    }
}
