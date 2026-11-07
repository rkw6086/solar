//! A warm, lower-register, analog-inspired electronic theme for Solar's beginning.
//!
//! The 112-BPM, 16-bar loop starts in E minor, opens into related major
//! harmonies, then returns through a suspended dominant. This is a musical
//! interpretation of emergence and renewal, not a claim about universal emotion.
//! Notes, releases and stereo echoes wrap around the same timeline. No
//! recordings or network assets are used. Detuned pads, rounded bass and dark
//! stereo echoes provide retro warmth without a high chiptune melody.
//! Tonic/dominant function reference:
//! https://musictheory.pugetsound.edu/mt21c/HarmonicFunction.html

use std::sync::LazyLock;

const SAMPLE_RATE: u32 = 22_050;
const CHANNELS: u16 = 2;
const BPM: f64 = 112.0;
const BEAT_SECONDS: f64 = 60.0 / BPM;
// Exactly 64 beats at 112 BPM: use samples, not rounded milliseconds.
const FRAME_COUNT: usize = 756_000;
const LOOP_SECONDS: f64 = FRAME_COUNT as f64 / SAMPLE_RATE as f64;
const HARMONIES: [[i32; 4]; 8] = [
    [52, 55, 59, 66], // Em(add9): E, G, B, F#.
    [52, 55, 59, 60], // Cmaj7/E: E, G, B, C.
    [50, 55, 57, 59], // G(add9): D, G, A, B.
    [50, 57, 62, 64], // Dsus2: D, A, D, E.
    [48, 52, 55, 59], // Am9: C, E, G, B above an A bass.
    [48, 52, 55, 59], // Cmaj7: C, E, G, B.
    [47, 52, 54, 57], // B7sus4: B, E, F#, A.
    [47, 51, 54, 57], // B7: B, D#, F#, A; D# resolves to E on the loop.
];
const BASS_NOTES: [i32; 8] = [40, 48, 43, 38, 45, 48, 47, 47];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Voice {
    Pad,
    Bass,
    Key,
    Kick,
    Brush,
}

struct Note {
    start: f64,
    length: f64,
    midi: f64,
    gain: f64,
    pan: f64,
    voice: Voice,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

/// A subtle, roughly 1.5 dB breath under the pulse, not a club-style pump.
fn sidechain(time: f64) -> f64 {
    let phase = (time / BEAT_SECONDS).rem_euclid(1.0);
    if phase < 0.06 {
        1.0 - 0.16 * smooth(phase / 0.06)
    } else {
        0.84 + 0.16 * smooth((phase - 0.06) / 0.52)
    }
}

fn score_notes() -> Vec<Note> {
    let mut notes = Vec::new();
    let pans = [-0.6, 0.4, -0.2, 0.6];
    // Quiet beginnings, a little opening in the middle, then a poised return.
    let warmth = [0.84, 0.92, 1.0, 0.98, 0.94, 1.0, 0.87, 0.84];
    let motif = [
        [59, 64],
        [60, 64],
        [59, 62],
        [57, 62],
        [59, 60],
        [55, 64],
        [54, 59],
        [57, 63],
    ];
    for (chord_index, chord) in HARMONIES.iter().enumerate() {
        let start = chord_index as f64 * 8.0 * BEAT_SECONDS;
        for (voice, midi) in chord.iter().enumerate() {
            notes.push(Note {
                start: start - 0.65,
                length: 8.0 * BEAT_SECONDS + 1.3,
                midi: f64::from(*midi),
                gain: 0.075 * warmth[chord_index],
                pan: pans[voice],
                voice: Voice::Pad,
            });
        }
        for beat in [0.0, 3.5, 6.0] {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: 0.82,
                midi: f64::from(BASS_NOTES[chord_index]),
                gain: 0.066,
                pan: 0.0,
                voice: Voice::Bass,
            });
        }
        for beat in 0..8 {
            notes.push(Note {
                start: start + f64::from(beat) * BEAT_SECONDS,
                length: 0.28,
                midi: 40.0,
                gain: if beat % 4 == 0 { 0.053 } else { 0.032 },
                pan: 0.0,
                voice: Voice::Kick,
            });
        }
        for beat in [2.0, 6.0] {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: 0.25,
                midi: 40.0,
                gain: 0.013,
                pan: 0.1,
                voice: Voice::Brush,
            });
        }
        // Slow inner movement stays within the chord's existing register.
        for (index, beat) in [1.0, 3.0, 5.0, 7.0].into_iter().enumerate() {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: 1.1,
                midi: f64::from(chord[[1, 2, 1, 3][index]]),
                gain: 0.021,
                pan: if index % 2 == 0 { -0.3 } else { 0.3 },
                voice: Voice::Key,
            });
        }
        for (index, beat) in [0.5, 4.5].into_iter().enumerate() {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: 1.8,
                midi: f64::from(motif[chord_index][index]),
                gain: 0.030 * warmth[chord_index],
                pan: 0.05,
                voice: Voice::Key,
            });
        }
    }
    notes
}

fn add_note(buffer: &mut [[f64; 2]], note: Note) {
    let frequency = 440.0 * 2.0_f64.powf((note.midi - 69.0) / 12.0);
    let start = (note.start.rem_euclid(LOOP_SECONDS) * f64::from(SAMPLE_RATE)).round() as usize;
    let frames = (note.length * f64::from(SAMPLE_RATE)).round() as usize;
    let left = ((1.0 - note.pan.clamp(-1.0, 1.0)) / 2.0).sqrt();
    let right = ((1.0 + note.pan.clamp(-1.0, 1.0)) / 2.0).sqrt();
    let mut filtered_noise = 0.0;
    for offset in 0..frames {
        let time = offset as f64 / f64::from(SAMPLE_RATE);
        let phase = std::f64::consts::TAU * frequency * time;
        let remaining = note.length - time;
        let sample = match note.voice {
            Voice::Pad => {
                let envelope = smooth(time / 1.05) * smooth(remaining / 1.05);
                let tone = 0.66 * phase.sin()
                    + 0.14 * (phase * 0.999).sin()
                    + 0.14 * (phase * 1.001).sin()
                    + 0.055 * (phase * 2.0).sin()
                    + 0.015 * (phase * 3.0).sin();
                envelope * tone
            }
            Voice::Bass => {
                let envelope =
                    smooth(time / 0.035) * smooth(remaining / 0.22) * (-time * 1.7).exp();
                envelope * (0.95 * phase.sin() + 0.05 * (phase * 2.0).sin())
            }
            Voice::Key => {
                let envelope =
                    smooth(time / 0.045) * smooth(remaining / 0.30) * (-time * 2.1).exp();
                envelope
                    * (0.93 * phase.sin() + 0.06 * (phase * 2.0).sin() + 0.01 * (phase * 3.0).sin())
            }
            Voice::Kick => {
                let envelope =
                    smooth(time / 0.014) * smooth(remaining / 0.12) * (-time * 14.0).exp();
                let sweep =
                    std::f64::consts::TAU * (52.0 * time + 0.7 * (1.0 - (-time * 20.0).exp()));
                envelope * sweep.sin()
            }
            Voice::Brush => {
                let bits = (offset as u32)
                    .wrapping_mul(747_796_405)
                    .wrapping_add(2_891_336_453);
                let noise = f64::from((bits ^ (bits >> 16)) & 0xffff) / 32767.5 - 1.0;
                filtered_noise += 0.12 * (noise - filtered_noise);
                smooth(time / 0.025)
                    * smooth(remaining / 0.09)
                    * (-time * 13.0).exp()
                    * filtered_noise
            }
        } * note.gain;
        let frame = &mut buffer[(start + offset) % FRAME_COUNT];
        frame[0] += sample * left;
        frame[1] += sample * right;
    }
}

fn render_score() -> Vec<i16> {
    let mut music = vec![[0.0_f64; 2]; FRAME_COUNT];
    let mut drums = vec![[0.0_f64; 2]; FRAME_COUNT];
    for note in score_notes() {
        let target = if matches!(note.voice, Voice::Kick | Voice::Brush) {
            &mut drums
        } else {
            &mut music
        };
        add_note(target, note);
    }
    let taps = [
        (BEAT_SECONDS * 0.75, 0.16),
        (BEAT_SECONDS * 1.5, 0.09),
        (BEAT_SECONDS * 2.25, 0.045),
    ]
    .map(|(delay, amount)| ((delay * f64::from(SAMPLE_RATE)).round() as usize, amount));
    let mut mix = Vec::with_capacity(FRAME_COUNT);
    for (index, frame) in music.iter().enumerate() {
        let duck = sidechain(index as f64 / f64::from(SAMPLE_RATE));
        let mut stereo = [0.0; 2];
        for (channel, direct) in frame.iter().enumerate() {
            let mut melodic = *direct;
            for (tap, (delay, amount)) in taps.iter().enumerate() {
                melodic += music[(index + FRAME_COUNT - delay) % FRAME_COUNT]
                    [if tap % 2 == 0 { 1 - channel } else { channel }]
                    * amount;
            }
            stereo[channel] = ((melodic * duck + drums[index][channel]) * 1.6).tanh();
        }
        mix.push(stereo);
    }
    // Two gentle low-pass stages take off upper harmonics. Warm the filters
    // over a full cycle before rendering so the loop has a periodic filter state.
    let alpha = 1.0 - (-std::f64::consts::TAU * 1_900.0 / f64::from(SAMPLE_RATE)).exp();
    let mut first = [0.0; 2];
    let mut second = [0.0; 2];
    let mut rendered = Vec::with_capacity(FRAME_COUNT * usize::from(CHANNELS));
    for pass in 0..2 {
        for frame in &mix {
            for channel in 0..2 {
                first[channel] += alpha * (frame[channel] - first[channel]);
                second[channel] += alpha * (first[channel] - second[channel]);
                if pass == 1 {
                    rendered.push(second[channel]);
                }
            }
        }
    }
    let peak = rendered
        .iter()
        .fold(0.0_f64, |peak, sample| peak.max(sample.abs()));
    let master = 0.72 / peak.max(f64::EPSILON);
    rendered
        .into_iter()
        .map(|sample| (sample * master * f64::from(i16::MAX)) as i16)
        .collect()
}

static SCORE_SAMPLES: LazyLock<Vec<i16>> = LazyLock::new(render_score);

/// Warm the deterministic synthesizer on a worker before starting native audio.
pub fn prepare() {
    LazyLock::force(&SCORE_SAMPLES);
}

/// Create a 34.286-second stereo, signed 16-bit PCM WAV at 22.05 kHz.
///
/// The 112-BPM E-minor theme moves from Em(add9) through warmer related
/// harmonies to B7sus4–B7, resolving home as it loops. Low-register pads,
/// sparse keys and a soft pulse leave room for the welcome UI. Rendering is cached once.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn synthesize_score(requested_volume: f32) -> Vec<u8> {
    let requested_gain = gain(requested_volume);
    let data_length = FRAME_COUNT as u32 * u32::from(CHANNELS) * 2;
    let mut wav = Vec::with_capacity(44 + data_length as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_length).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * u32::from(CHANNELS) * 2).to_le_bytes());
    wav.extend_from_slice(&(CHANNELS * 2).to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_length.to_le_bytes());
    for sample in SCORE_SAMPLES.iter() {
        wav.extend_from_slice(&((f64::from(*sample) * requested_gain) as i16).to_le_bytes());
    }
    wav
}

/// Begin or continue the same loop. Call on the AppKit main thread.
///
/// For a gentle entrance, call `play(0.0)` followed by a short series of
/// `set_volume` calls. Changes do not regenerate PCM or restart the timeline.
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
        let data = NSData::with_bytes(&synthesize_score(1.0));
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

    fn samples(wav: &[u8]) -> Vec<i16> {
        wav[44..]
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
            .collect()
    }

    #[test]
    fn background_pulse_is_periodic_and_only_gently_ducks() {
        assert!((BEAT_SECONDS * 64.0 - LOOP_SECONDS).abs() < 1e-12);
        assert!((sidechain(0.0) - 1.0).abs() < 1e-12);
        assert!((sidechain(BEAT_SECONDS * 0.06) - 0.84).abs() < 1e-12);
        for sample in 0..1000 {
            let time = sample as f64 * BEAT_SECONDS / 1000.0;
            assert!((0.84..=1.0).contains(&sidechain(time)));
            assert!((sidechain(time) - sidechain(time + BEAT_SECONDS * 4.0)).abs() < 1e-12);
        }
        assert!((sidechain(LOOP_SECONDS - 1e-6) - sidechain(0.0)).abs() < 1e-8);
    }

    #[test]
    fn background_notes_stay_low_and_dominant_returns_to_minor_home() {
        let notes = score_notes();
        let pitched: Vec<_> = notes
            .iter()
            .filter(|note| !matches!(note.voice, Voice::Kick | Voice::Brush))
            .collect();
        assert!(pitched.iter().all(|note| note.midi <= 66.0));
        assert!(
            pitched
                .iter()
                .filter(|note| note.voice == Voice::Key)
                .count()
                < 60
        );
        assert_eq!(HARMONIES[0], [52, 55, 59, 66]); // E minor + ninth.
        assert!(HARMONIES[6].contains(&52)); // The E suspension.
        assert!(HARMONIES[7].contains(&51)); // D# resolves up to E.
        assert_eq!(BASS_NOTES.first(), Some(&40));
        assert_eq!(BASS_NOTES.last(), Some(&47)); // Dominant B returns to E.
    }

    #[test]
    fn valid_stereo_score_has_a_fixed_duration_and_bounded_size() {
        prepare();
        let wav = synthesize_score(1.0);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(
            u16::from_le_bytes(wav[22..24].try_into().unwrap()),
            CHANNELS
        );
        assert_eq!(
            u32::from_le_bytes(wav[24..28].try_into().unwrap()),
            SAMPLE_RATE
        );
        assert_eq!(samples(&wav).len(), FRAME_COUNT * usize::from(CHANNELS));
        assert!(wav.len() < 6 * 1024 * 1024);
    }

    #[test]
    fn repeats_are_deterministic_and_volume_is_bounded() {
        let full = synthesize_score(1.0);
        let quiet = synthesize_score(0.25);
        assert_eq!(full, synthesize_score(2.0));
        assert_eq!(quiet, synthesize_score(0.25));
        let full = samples(&full);
        let quiet = samples(&quiet);
        let peak = full
            .iter()
            .map(|sample| i32::from(*sample).abs())
            .max()
            .unwrap();
        assert!(peak > 1000 && peak < i32::from(i16::MAX));
        assert!(
            full.iter()
                .zip(&quiet)
                .all(|(a, b)| (i32::from(*a) / 4 - i32::from(*b)).abs() <= 1)
        );
        for volume in [0.0, -0.5, f32::NAN, f32::INFINITY] {
            assert!(
                samples(&synthesize_score(volume))
                    .iter()
                    .all(|sample| *sample == 0)
            );
        }
    }

    #[test]
    fn loop_boundary_has_no_abrupt_amplitude_or_slope_jump() {
        let samples = samples(&synthesize_score(1.0));
        for channel in 0..usize::from(CHANNELS) {
            let values: Vec<i32> = samples
                .iter()
                .skip(channel)
                .step_by(usize::from(CHANNELS))
                .map(|sample| i32::from(*sample))
                .collect();
            let seam = values[0] - values[values.len() - 1];
            let before = values[values.len() - 1] - values[values.len() - 2];
            let after = values[1] - values[0];
            // Bright synth harmonics naturally have larger per-sample steps
            // than the former pad score. A click is a discontinuity relative
            // to the slopes on either side, not a large continuous slope.
            assert!(seam.abs() <= before.abs().max(after.abs()) + 300);
            assert!(
                (seam - before).abs() < 300,
                "before seam slope {before} -> {seam}"
            );
            assert!(
                (after - seam).abs() < 300,
                "after seam slope {seam} -> {after}"
            );
            assert!(values.iter().any(|sample| *sample != 0));
        }
    }
}
