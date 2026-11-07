//! A short, syncopated synth motif for Solar's beginning.
//!
//! The 112-BPM, 12-bar loop follows i–iv–III–VI–iiø7–V7 in E minor.
//! Chords alternate six and two beats, then the progression repeats with a
//! varied response. Clipped foreground notes follow those changes; only a
//! quiet root/fifth layer sustains in the bass.
//! Notes, releases and short pluck echoes wrap around the same timeline.
//! All sounds are synthesized locally, with no recordings or network assets.
//! Envelope reference: https://learningsynths.ableton.com/recipes/plucked-bass
//! Harmony reference: https://musictheory.pugetsound.edu/mt21c/DiatonicChordsInMinor.html

use std::sync::LazyLock;

const SAMPLE_RATE: u32 = 22_050;
const CHANNELS: u16 = 2;
const BPM: f64 = 112.0;
const BEAT_SECONDS: f64 = 60.0 / BPM;
// Exactly 48 beats at 112 BPM: use samples, not rounded milliseconds.
const FRAME_COUNT: usize = 567_000;
const LOOP_SECONDS: f64 = FRAME_COUNT as f64 / SAMPLE_RATE as f64;
// Root movement 1–4–3–6–2, followed by V7 to turn back to the tonic.
// The melodic motifs imply the extensions; these chords are never held as pads.
const HARMONIES: [[i32; 4]; 6] = [
    [52, 55, 59, 54], // Em(add9): E, G, B, F#.
    [57, 60, 64, 59], // Am(add9): A, C, E, B.
    [55, 59, 62, 54], // Gmaj7: G, B, D, F#.
    [48, 52, 55, 59], // Cmaj7: C, E, G, B.
    [54, 57, 60, 52], // F#m7b5: F#, A, C, E.
    [47, 51, 54, 57], // B7: B, D#, F#, A; D# resolves up to E.
];
const BASS_NOTES: [i32; 6] = [40, 33, 31, 36, 30, 35];
const LOW_FIFTHS: [i32; 6] = [35, 40, 38, 43, 36, 42]; // F# uses its diminished fifth, C.
const CHORD_START_BEATS: [f64; 6] = [0.0, 6.0, 8.0, 14.0, 16.0, 22.0];
const CHORD_LENGTH_BEATS: [f64; 6] = [6.0, 2.0, 6.0, 2.0, 6.0, 2.0];
// Each row spans two chords: six beats of call, then a two-beat answer.
const MELODY: [[i32; 8]; 3] = [
    [52, 55, 59, 54, 55, 59, 60, 59], // Em(add9), then Am(add9).
    [55, 59, 62, 54, 59, 62, 59, 52], // Gmaj7, then Cmaj7.
    [54, 57, 60, 52, 57, 60, 57, 51], // F#m7b5, then B7.
];
const MELODY_BEATS: [f64; 8] = [0.5, 1.25, 2.75, 3.5, 4.25, 5.5, 6.25, 7.25];
const MELODY_LENGTHS: [f64; 8] = [0.18, 0.14, 0.22, 0.16, 0.18, 0.14, 0.20, 0.16];
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
    (6.0, 0.48, 0.075),
    (7.5, 0.35, 0.070),
];
const SNARE_LAG_SECONDS: f64 = 0.012;
const SHAKER_SWING: f64 = 0.56;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Voice {
    LowBed,
    Bass,
    Pluck,
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

// Zero sustain: the body decays even before the explicit note release. The
// short final taper keeps a clipped note from making a digital discontinuity.
fn pluck_envelope(time: f64, length: f64, velocity: f64) -> f64 {
    if time < 0.0 || time >= length {
        return 0.0;
    }
    let decay = 28.0 - 10.0 * velocity.clamp(0.0, 1.0);
    smooth(time / 0.003) * (-time * decay).exp() * smooth((length - time) / 0.03)
}

/// A short breath follows actual kicks, including the offbeats. Empty quarters
/// stay unducked; the background never acquires a four-on-the-floor pump.
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

fn harmonic_index(beat: f64) -> usize {
    let phase = beat.rem_euclid(24.0);
    CHORD_START_BEATS
        .iter()
        .rposition(|start| phase >= *start)
        .unwrap_or(0)
}

fn score_notes() -> Vec<Note> {
    let mut notes = Vec::new();
    for cycle in 0..2 {
        for chord_index in 0..HARMONIES.len() {
            let start = (cycle as f64 * 24.0 + CHORD_START_BEATS[chord_index]) * BEAT_SECONDS;
            let root = BASS_NOTES[chord_index];
            // Only roots and fifths sustain, entirely below the foreground.
            // Their fades finish on the actual 6/2-beat harmonic boundaries.
            let fifth = LOW_FIFTHS[chord_index];
            for (midi, level, pan) in [(root, 0.008, -0.08), (fifth, 0.004, 0.08)] {
                notes.push(Note {
                    start,
                    length: CHORD_LENGTH_BEATS[chord_index] * BEAT_SECONDS,
                    midi: f64::from(midi),
                    gain: level,
                    pan,
                    voice: Voice::LowBed,
                });
            }
        }
    }
    for phrase in 0..6 {
        let start_beat = phrase as f64 * GROOVE_BEATS;
        let start = start_beat * BEAT_SECONDS;
        for (beat, length, velocity) in BASS_PATTERN {
            let chord_index = harmonic_index(start_beat + beat);
            let phase = (start_beat + beat).rem_euclid(24.0);
            let beats_to_change =
                CHORD_START_BEATS[chord_index] + CHORD_LENGTH_BEATS[chord_index] - phase;
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: length.min(beats_to_change) * BEAT_SECONDS,
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
        // Six beats of the first chord, then two beats of its answering chord.
        // All foreground attacks are offbeat and every note ends before the
        // next harmonic boundary. The second pass changes one call note.
        let accents = [1.0, 0.70, 0.88, 0.79, 0.91, 0.72, 0.85, 0.64];
        let touch = [-0.006, 0.008, -0.004, 0.011, -0.003, 0.007, 0.012, -0.002];
        let pans = [-0.12, -0.04, 0.09, -0.07, 0.14, 0.03, 0.08, -0.10];
        for (index, original_beat) in MELODY_BEATS.into_iter().enumerate() {
            let melody = MELODY[phrase % MELODY.len()];
            let returning = phrase >= 3;
            let beat = original_beat
                + if returning {
                    match index {
                        1 | 4 | 6 => 0.125,
                        5 => -0.25,
                        _ => 0.0,
                    }
                } else {
                    0.0
                };
            let midi = match (returning, index) {
                (true, 4) => melody[2],
                (true, 5) => melody[1],
                (true, 6) => [64, 55, 54][phrase % 3],
                (true, 7) => [60, 52, 51][phrase % 3],
                _ => melody[index],
            };
            let velocity = accents[index] * if returning { 0.96 } else { 1.0 };
            notes.push(Note {
                start: start
                    + beat * BEAT_SECONDS
                    + touch[index] * if phrase % 2 == 0 { 1.0 } else { 0.8 },
                length: MELODY_LENGTHS[index] + 0.005 * velocity,
                midi: f64::from(midi),
                gain: 0.28 * velocity,
                pan: pans[index],
                voice: Voice::Pluck,
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
            Voice::LowBed => {
                let envelope = smooth(time / 0.65) * smooth(remaining / 0.65);
                envelope * (0.97 * phase.sin() + 0.03 * (phase * 2.0).sin())
            }
            Voice::Bass => {
                let envelope =
                    smooth(time / 0.018) * smooth(remaining / 0.095) * (-time * 1.9).exp();
                envelope * (0.95 * phase.sin() + 0.05 * (phase * 2.0).sin())
            }
            Voice::Pluck => {
                // A rounded mallet-like attack closes quickly into the body.
                // The upper partials decay faster than the fundamental, unlike
                // the fixed harmonic balance of a held organ tone.
                let velocity = (note.gain / 0.28).clamp(0.0, 1.0);
                let transient = (-time * (55.0 - 20.0 * velocity)).exp();
                let tone = (phase + (0.22 + 0.16 * velocity) * transient * (phase * 2.0).sin())
                    .sin()
                    + (0.18 + 0.12 * velocity) * (-time * 35.0).exp() * (phase * 2.0).sin()
                    + 0.10 * (-time * 60.0).exp() * (phase * 3.0).sin();
                pluck_envelope(time, note.length, velocity) * tone
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
    let mut lead = vec![[0.0_f64; 2]; FRAME_COUNT];
    let mut background = vec![[0.0_f64; 2]; FRAME_COUNT];
    let mut drums = vec![[0.0_f64; 2]; FRAME_COUNT];
    for note in score_notes() {
        let target = if note.voice.is_percussion() {
            &mut drums
        } else if note.voice == Voice::Pluck {
            &mut lead
        } else {
            &mut background
        };
        add_note(target, note);
    }
    // Short, low-level reflections belong only to the plucks. The low bed and
    // bass stay dry so overlapping echoes cannot turn into another organ wash.
    let taps = [(BEAT_SECONDS * 0.5, 0.10), (BEAT_SECONDS * 0.75, 0.045)]
        .map(|(delay, amount)| ((delay * f64::from(SAMPLE_RATE)).round() as usize, amount));
    let mut mix = Vec::with_capacity(FRAME_COUNT);
    for (index, frame) in lead.iter().enumerate() {
        let duck = sidechain(index as f64 / f64::from(SAMPLE_RATE));
        let mut stereo = [0.0; 2];
        for (channel, direct) in frame.iter().enumerate() {
            let mut melodic = *direct;
            for (tap, (delay, amount)) in taps.iter().enumerate() {
                melodic += lead[(index + FRAME_COUNT - delay) % FRAME_COUNT]
                    [if tap % 2 == 0 { 1 - channel } else { channel }]
                    * amount;
            }
            stereo[channel] =
                (((melodic + background[index][channel]) * duck + drums[index][channel]) * 1.6)
                    .tanh();
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

/// Create a 25.714-second stereo, signed 16-bit PCM WAV at 22.05 kHz.
///
/// The 112-BPM E-minor theme moves through i–iv–III–VI–iiø7–V7, with a six-note
/// call and two-note reply. Short foreground plucks, a quiet bass foundation,
/// and a softly swung beat leave room for the welcome UI. Rendering is cached.
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
        assert!((BEAT_SECONDS * 48.0 - LOOP_SECONDS).abs() < 1e-12);
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
    fn only_low_roots_and_fifths_sustain_under_the_new_progression() {
        let notes = score_notes();
        let pitched: Vec<_> = notes
            .iter()
            .filter(|note| !note.voice.is_percussion())
            .collect();
        assert!(pitched.iter().all(|note| note.midi <= 64.0));
        let beds: Vec<_> = pitched
            .iter()
            .filter(|note| note.voice == Voice::LowBed)
            .collect();
        assert_eq!(beds.len(), 24); // Two voices, six chords, two passes.
        assert!(beds.iter().all(|note| (28.0..=43.0).contains(&note.midi)));
        assert!(beds.iter().all(|note| note.gain <= 0.008));
        assert!(
            pitched
                .iter()
                .all(|note| note.length <= BEAT_SECONDS || note.voice == Voice::LowBed)
        );
        let roots: Vec<_> = BASS_NOTES.iter().map(|note| note % 12).collect();
        assert_eq!(roots, [4, 9, 7, 0, 6, 11]); // 1–4–3–6–2, then dominant.
        assert_eq!(LOW_FIFTHS[4] % 12, 0); // Diminished fifth C over F#.
        assert!(HARMONIES[5].contains(&51)); // D# resolves up to E.
        assert_eq!(MELODY[2][7] + 1, MELODY[0][0]);
    }

    #[test]
    fn harmony_alternates_six_and_two_beats_with_bass_and_melody_aligned() {
        let notes = score_notes();
        assert_eq!(CHORD_START_BEATS, [0.0, 6.0, 8.0, 14.0, 16.0, 22.0]);
        assert_eq!(CHORD_LENGTH_BEATS, [6.0, 2.0, 6.0, 2.0, 6.0, 2.0]);
        for cycle in 0..2 {
            for chord in 0..6 {
                let start = cycle as f64 * 24.0 + CHORD_START_BEATS[chord];
                let end = start + CHORD_LENGTH_BEATS[chord];
                assert_eq!(harmonic_index(start), chord);
                assert_eq!(harmonic_index(end - 0.001), chord);
                assert!(notes.iter().any(|note| note.voice == Voice::Bass
                    && (note.start / BEAT_SECONDS - start).abs() < 1e-9
                    && note.midi == f64::from(BASS_NOTES[chord])));
                let melody: Vec<_> = notes
                    .iter()
                    .filter(|note| {
                        let beat = note.start / BEAT_SECONDS;
                        note.voice == Voice::Pluck && beat >= start && beat < end
                    })
                    .collect();
                assert_eq!(melody.len(), if chord % 2 == 0 { 6 } else { 2 });
                assert!(melody.iter().all(|note| {
                    HARMONIES[chord]
                        .iter()
                        .any(|pitch| *pitch % 12 == note.midi as i32 % 12)
                        && note.start + note.length <= end * BEAT_SECONDS
                }));
            }
        }
        for note in notes.iter().filter(|note| note.voice == Voice::Bass) {
            let beat = note.start / BEAT_SECONDS;
            let chord = harmonic_index(beat);
            let phase = beat.rem_euclid(24.0);
            let available = CHORD_START_BEATS[chord] + CHORD_LENGTH_BEATS[chord] - phase;
            assert!(note.length <= available * BEAT_SECONDS + 1e-9);
        }
    }

    #[test]
    fn foreground_is_short_syncopated_plucks_with_space_and_no_sustain() {
        let notes = score_notes();
        let plucks: Vec<_> = notes
            .iter()
            .filter(|note| note.voice == Voice::Pluck)
            .collect();
        assert_eq!(plucks.len(), 48);
        assert!(
            plucks
                .iter()
                .all(|note| (0.14..=0.23).contains(&note.length))
        );
        assert!(plucks.iter().all(|note| {
            let beat = note.start / BEAT_SECONDS;
            (beat - beat.round()).abs() > 0.1
        }));
        for pair in plucks.windows(2) {
            assert!(pair[1].start - pair[0].start - pair[0].length >= 0.15);
        }
        let duty: f64 = plucks.iter().map(|note| note.length).sum::<f64>() / LOOP_SECONDS;
        assert!(duty < 0.35);
        assert!(pluck_envelope(0.12, 0.22, 1.0) < pluck_envelope(0.012, 0.22, 1.0) * 0.15);
        assert_eq!(pluck_envelope(0.22, 0.22, 1.0), 0.0);
        assert_eq!(pluck_envelope(0.30, 0.22, 1.0), 0.0);
        assert!(pluck_envelope(0.12, 0.22, 0.6) < pluck_envelope(0.12, 0.22, 1.0));
        assert_ne!(plucks[4].midi, plucks[28].midi); // Second-pass contour varies.
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
        assert!((0.07..0.22).contains(&rms), "full-volume RMS {rms}");
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

        let mut lead = vec![[0.0_f64; 2]; FRAME_COUNT];
        let mut bed = vec![[0.0_f64; 2]; FRAME_COUNT];
        let mut drums = vec![[0.0_f64; 2]; FRAME_COUNT];
        for note in score_notes() {
            let target = if note.voice.is_percussion() {
                &mut drums
            } else if note.voice == Voice::Pluck {
                &mut lead
            } else if note.voice == Voice::LowBed {
                &mut bed
            } else {
                continue;
            };
            add_note(target, note);
        }
        let energy = |stem: &[[f64; 2]]| -> f64 {
            stem.iter()
                .flatten()
                .map(|sample| sample * sample)
                .sum::<f64>()
        };
        let ratio = (energy(&drums) / energy(&lead)).sqrt();
        assert!(
            (0.20..1.50).contains(&ratio),
            "percussion/foreground RMS ratio {ratio}"
        );
        let lead_to_bed = (energy(&lead) / energy(&bed)).sqrt();
        assert!(lead_to_bed > 4.0, "foreground/bed RMS ratio {lead_to_bed}");
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
