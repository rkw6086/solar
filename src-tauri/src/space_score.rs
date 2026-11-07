//! A warm, lower-register, analog-inspired electronic theme for Solar's beginning.
//!
//! The 112-BPM, 16-bar loop starts in E minor, opens into related major
//! harmonies, then returns through a suspended dominant. This is a musical
//! interpretation of emergence and renewal, not a claim about universal emotion.
//! Notes, releases and stereo echoes wrap around the same timeline. No
//! recordings or network assets are used. Detuned pads, rounded bass and dark
//! stereo echoes provide retro warmth without a high chiptune melody. A broken
//! two-bar beat, late soft snares and swung shakers give the background motion.
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
const GROOVE_BEATS: f64 = 8.0;
// Two bars of half-time movement: the second bar answers the first rather
// than repeating a kick on every quarter. Gains also drive the gentle ducking.
const KICK_PATTERN: [(f64, f64); 6] = [
    (0.0, 0.155),
    (1.5, 0.108),
    (3.25, 0.092),
    (4.0, 0.142),
    (5.75, 0.104),
    (7.5, 0.080),
];
// A short bass answer follows some kicks and anticipates the final one.
// Tuples are beat, duration in beats, and gain. No upper-octave lead is added.
const BASS_PATTERN: [(f64, f64, f64); 7] = [
    (0.0, 0.95, 0.105),
    (1.5, 0.52, 0.082),
    (3.25, 0.48, 0.078),
    (4.0, 0.90, 0.100),
    (5.75, 0.48, 0.080),
    (7.0, 0.28, 0.057),
    (7.5, 0.35, 0.070),
];
const SNARE_LAG_SECONDS: f64 = 0.012;
const SHAKER_SWING: f64 = 0.56;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Voice {
    Pad,
    Bass,
    Key,
    Kick,
    Snare,
    Brush,
    Shaker,
}

impl Voice {
    fn is_percussion(self) -> bool {
        matches!(self, Self::Kick | Self::Snare | Self::Brush | Self::Shaker)
    }
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

/// A short breath follows actual kicks, including the offbeats. Empty quarters
/// stay unducked; the pads never acquire a four-on-the-floor pumping rhythm.
fn sidechain(time: f64) -> f64 {
    let phase = (time / BEAT_SECONDS).rem_euclid(GROOVE_BEATS);
    let mut duck = 1.0_f64;
    for (beat, velocity) in KICK_PATTERN {
        let age = (phase - beat).rem_euclid(GROOVE_BEATS) * BEAT_SECONDS;
        let depth = 0.18 * velocity / KICK_PATTERN[0].1;
        let pulse = if age < 0.012 {
            smooth(age / 0.012)
        } else {
            1.0 - smooth((age - 0.012) / 0.23)
        };
        duck = duck.min(1.0 - depth * pulse);
    }
    duck
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
        let start = chord_index as f64 * GROOVE_BEATS * BEAT_SECONDS;
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
        for (beat, length, velocity) in BASS_PATTERN {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: length * BEAT_SECONDS,
                midi: f64::from(BASS_NOTES[chord_index]),
                gain: velocity,
                pan: 0.0,
                voice: Voice::Bass,
            });
        }
        for (beat, velocity) in KICK_PATTERN {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: 0.28,
                midi: 40.0,
                gain: velocity,
                pan: 0.0,
                voice: Voice::Kick,
            });
        }
        for beat in [2.0, 6.0] {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS + SNARE_LAG_SECONDS,
                length: 0.24,
                midi: 40.0,
                gain: if beat < 4.0 { 0.138 } else { 0.148 },
                pan: 0.04,
                voice: Voice::Snare,
            });
        }
        for (beat, velocity) in [(1.75, 0.034), (5.5, 0.026), (6.75, 0.030)] {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: 0.11,
                midi: 40.0,
                gain: velocity,
                pan: -0.13,
                voice: Voice::Brush,
            });
        }
        // Each offbeat is a little late (56% swing). Alternate accents and a
        // quiet second-bar variation keep these dark ticks out of the foreground.
        let shaker_accents = [0.45, 1.0, 0.34, 0.78, 0.48, 0.86, 0.30, 0.94];
        for step in 0..16 {
            let beat = (step / 2) as f64 + if step % 2 == 0 { 0.0 } else { SHAKER_SWING };
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: 0.075,
                midi: 40.0,
                gain: 0.060 * shaker_accents[step % 8] * if step >= 8 { 0.92 } else { 1.0 },
                pan: if step % 2 == 0 { -0.24 } else { 0.24 },
                voice: Voice::Shaker,
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
    let mut slow_noise = 0.0;
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
                    smooth(time / 0.018) * smooth(remaining / 0.095) * (-time * 1.9).exp();
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
            Voice::Snare | Voice::Brush | Voice::Shaker => {
                let bits = (offset as u32)
                    .wrapping_mul(747_796_405)
                    .wrapping_add((start as u32).wrapping_mul(2_891_336_453));
                let noise = f64::from((bits ^ (bits >> 16)) & 0xffff) / 32767.5 - 1.0;
                // No open hi-hat: restrict the noise before the master low-pass.
                filtered_noise += 0.22 * (noise - filtered_noise);
                slow_noise += 0.065 * (filtered_noise - slow_noise);
                if note.voice == Voice::Shaker {
                    smooth(time / 0.006)
                        * smooth(remaining / 0.032)
                        * (-time * 40.0).exp()
                        * (filtered_noise - slow_noise)
                } else {
                    let ghost = note.voice == Voice::Brush;
                    let envelope = smooth(time / 0.005)
                        * smooth(remaining / 0.065)
                        * (-time * if ghost { 30.0 } else { 20.0 }).exp();
                    let body_phase =
                        std::f64::consts::TAU * (168.0 * time + 0.6 * (1.0 - (-time * 25.0).exp()));
                    envelope
                        * (0.62 * body_phase.sin()
                            + 0.13
                                * (std::f64::consts::TAU * 330.0 * time).sin()
                                * (-time * 30.0).exp()
                            + 0.8 * filtered_noise)
                }
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
        let target = if note.voice.is_percussion() {
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
/// sparse keys and a softly swung half-time beat leave room for the welcome UI.
/// Rendering is cached once.
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
    fn background_pulse_follows_broken_kicks_instead_of_every_quarter() {
        assert!((BEAT_SECONDS * 64.0 - LOOP_SECONDS).abs() < 1e-12);
        assert!((sidechain(0.0) - 1.0).abs() < 1e-12);
        assert!((sidechain(0.012) - 0.82).abs() < 1e-12);
        // Beat two is empty, while the following offbeat breathes with its kick.
        assert_eq!(sidechain(BEAT_SECONDS * 1.0 + 0.012), 1.0);
        assert!(sidechain(BEAT_SECONDS * 1.5 + 0.012) < 0.9);
        for sample in 0..1000 {
            let time = sample as f64 * BEAT_SECONDS * GROOVE_BEATS / 1000.0;
            assert!((0.82..=1.0).contains(&sidechain(time)));
            assert!(
                (sidechain(time) - sidechain(time + BEAT_SECONDS * GROOVE_BEATS)).abs() < 1e-12
            );
        }
        assert!((sidechain(LOOP_SECONDS - 1e-6) - sidechain(0.0)).abs() < 1e-8);
    }

    #[test]
    fn groove_has_syncopated_kicks_late_half_time_backbeats_and_ghosts() {
        let notes = score_notes();
        let first_phrase: Vec<_> = notes
            .iter()
            .filter(|note| note.start >= 0.0 && note.start < 8.0 * BEAT_SECONDS)
            .collect();
        let kicks: Vec<_> = first_phrase
            .iter()
            .filter(|note| note.voice == Voice::Kick)
            .collect();
        assert_eq!(kicks.len(), 6);
        let offbeat_count = kicks
            .iter()
            .filter(|note| {
                let beat = note.start / BEAT_SECONDS;
                (beat - beat.round()).abs() > 0.1
            })
            .count();
        assert_eq!(offbeat_count, 4);
        assert!(kicks[0].gain > kicks[1].gain);
        assert!(kicks[3].gain > kicks[4].gain);
        let backbeats: Vec<_> = first_phrase
            .iter()
            .filter(|note| note.voice == Voice::Snare)
            .collect();
        assert_eq!(backbeats.len(), 2);
        for (note, beat) in backbeats.iter().zip([2.0, 6.0]) {
            let lag = note.start - beat * BEAT_SECONDS;
            assert!((0.008..=0.018).contains(&lag));
        }
        let ghosts: Vec<_> = first_phrase
            .iter()
            .filter(|note| note.voice == Voice::Brush)
            .collect();
        assert!(ghosts.len() >= 2);
        assert!(
            ghosts
                .iter()
                .all(|note| note.gain < backbeats[0].gain * 0.3)
        );
        let bass: Vec<_> = first_phrase
            .iter()
            .filter(|note| note.voice == Voice::Bass)
            .collect();
        assert!(kicks.iter().all(|kick| {
            bass.iter()
                .any(|note| (note.start - kick.start).abs() < 1e-9)
        }));
        assert!(bass.iter().all(|note| note.length < BEAT_SECONDS));
    }

    #[test]
    fn quiet_shakers_swing_instead_of_playing_straight_eighths() {
        let notes = score_notes();
        let shakers: Vec<_> = notes
            .iter()
            .filter(|note| note.voice == Voice::Shaker && note.start < 8.0 * BEAT_SECONDS)
            .collect();
        assert_eq!(shakers.len(), 16);
        for pair in shakers.chunks_exact(2) {
            let offbeat = (pair[1].start - pair[0].start) / BEAT_SECONDS;
            assert!((0.54..=0.58).contains(&offbeat));
            assert!(pair[1].gain > pair[0].gain);
            assert!(
                pair.iter()
                    .all(|note| note.gain <= 0.06 && note.length <= 0.08)
            );
        }
        assert!(shakers[8].gain < shakers[0].gain);
    }

    #[test]
    fn background_notes_stay_low_and_dominant_returns_to_minor_home() {
        let notes = score_notes();
        let pitched: Vec<_> = notes
            .iter()
            .filter(|note| !note.voice.is_percussion())
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
    fn output_has_headroom_and_stays_dark_with_audible_percussion() {
        let output = samples(&synthesize_score(1.0));
        let normalized: Vec<_> = output
            .iter()
            .map(|sample| f64::from(*sample) / f64::from(i16::MAX))
            .collect();
        let rms = (normalized.iter().map(|sample| sample * sample).sum::<f64>()
            / normalized.len() as f64)
            .sqrt();
        let peak = normalized
            .iter()
            .fold(0.0_f64, |peak, sample| peak.max(sample.abs()));
        assert!((0.10..0.32).contains(&rms), "full-volume RMS {rms}");
        assert!((0.71..0.73).contains(&peak));
        // First differences emphasize upper frequencies. This bound catches
        // accidentally bright noise/lead additions without testing a recipe.
        let difference_rms = (normalized
            .iter()
            .zip(normalized.iter().skip(2))
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            / (normalized.len() - 2) as f64)
            .sqrt();
        assert!(difference_rms / rms < 0.18);

        let mut music = vec![[0.0_f64; 2]; FRAME_COUNT];
        let mut drums = vec![[0.0_f64; 2]; FRAME_COUNT];
        for note in score_notes() {
            let target = if note.voice.is_percussion() {
                &mut drums
            } else {
                &mut music
            };
            add_note(target, note);
        }
        let energy = |stem: &[[f64; 2]]| -> f64 {
            stem.iter()
                .flatten()
                .map(|sample| sample * sample)
                .sum::<f64>()
        };
        let ratio = (energy(&drums) / energy(&music)).sqrt();
        assert!(
            (0.12..0.50).contains(&ratio),
            "percussion/music RMS ratio {ratio}"
        );
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
            // A click is a discontinuity relative to neighboring slopes,
            // rather than the ordinary sample-to-sample movement of a tone.
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
