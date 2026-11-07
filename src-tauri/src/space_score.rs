//! A warm, upbeat synthwave theme for Solar's beginning.
//!
//! A steady 120-BPM backbeat and pulsing octave bass support an analog-style
//! synth hook. Chords retain the user's six/two-beat harmonic rhythm, with a
//! varied second pass. The foreground releases between phrases; a quiet bass
//! foundation supplies the only sustained layer. The master level is modest.
//! All sounds are synthesized locally, with no recordings or network assets.
//! Palette reference: https://articles.roland.com/synthwave-essentials-for-the-jupiter-xm/

use std::sync::LazyLock;

const SAMPLE_RATE: u32 = 22_050;
const CHANNELS: u16 = 2;
const BPM: f64 = 120.0;
const BEAT_SECONDS: f64 = 60.0 / BPM;
const FRAME_COUNT: usize = 529_200; // Exactly 48 beats, 24 seconds.
const LOOP_SECONDS: f64 = FRAME_COUNT as f64 / SAMPLE_RATE as f64;
const MASTER_PEAK: f64 = 0.45;
const GROOVE_BEATS: f64 = 8.0;
// Em(add9), Am(add9), Gmaj7, Cmaj7, F#m7b5, B7.
const HARMONIES: [[i32; 4]; 6] = [
    [52, 55, 59, 54],
    [57, 60, 64, 59],
    [55, 59, 62, 54],
    [48, 52, 55, 59],
    [54, 57, 60, 52],
    [47, 51, 54, 57],
];
const BASS_NOTES: [i32; 6] = [40, 45, 43, 36, 42, 35];
const BED_NOTES: [i32; 6] = [28, 33, 31, 36, 30, 35];
const LOW_FIFTHS: [i32; 6] = [35, 40, 38, 43, 36, 42];
const CHORD_START_BEATS: [f64; 6] = [0.0, 6.0, 8.0, 14.0, 16.0, 22.0];
const CHORD_LENGTH_BEATS: [f64; 6] = [6.0, 2.0, 6.0, 2.0, 6.0, 2.0];
const MELODY: [[i32; 8]; 3] = [
    [64, 67, 71, 67, 66, 64, 64, 60],
    [62, 66, 67, 71, 69, 67, 67, 64],
    [66, 69, 72, 69, 64, 66, 66, 63],
];
const RESPONSE_MELODY: [[i32; 8]; 3] = [
    [64, 67, 71, 69, 67, 66, 60, 64],
    [62, 66, 67, 69, 71, 67, 64, 67],
    [66, 69, 72, 71, 69, 66, 63, 66],
];
const MELODY_BEATS: [f64; 8] = [0.5, 1.25, 2.0, 3.25, 4.0, 5.0, 6.25, 7.25];
const MELODY_LENGTHS: [f64; 8] = [0.28, 0.32, 0.42, 0.30, 0.36, 0.34, 0.30, 0.32];
const KICK_PATTERN: [(f64, f64); 8] = [
    (0.0, 0.17),
    (1.0, 0.14),
    (2.0, 0.15),
    (3.0, 0.14),
    (4.0, 0.17),
    (5.0, 0.14),
    (6.0, 0.15),
    (7.0, 0.14),
];
const SNARE_LAG_SECONDS: f64 = 0.006;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Voice {
    LowBed,
    Bass,
    Lead,
    Chord,
    Kick,
    Snare,
    Hat,
}

impl Voice {
    fn is_percussion(self) -> bool {
        matches!(self, Self::Kick | Self::Snare | Self::Hat)
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

fn lead_envelope(time: f64, length: f64, velocity: f64) -> f64 {
    if time < 0.0 || time >= length {
        return 0.0;
    }
    smooth(time / 0.009)
        * (-time * (8.5 - 2.0 * velocity.clamp(0.0, 1.0))).exp()
        * smooth((length - time) / 0.075)
}

// The source is a small bank of band-limited saw partials. Detuning and a
// closing spectral envelope give motion and warmth rather than a static organ.
fn analog_tone(phase: f64, rolloff: f64, detune: bool) -> f64 {
    let mut tone = 0.0;
    for partial in 1..=8 {
        let harmonic = f64::from(partial);
        let weight = (-harmonic * rolloff).exp() / harmonic;
        let oscillator = if detune {
            0.62 * (phase * harmonic).sin() + 0.38 * (phase * harmonic * 1.0045 + 0.12).sin()
        } else {
            // A small odd-harmonic pulse component rounds the saw bass.
            (phase * harmonic).sin() * if partial % 2 == 1 { 1.0 } else { 0.55 }
        };
        tone += weight * oscillator;
    }
    tone * 0.9
}

fn sidechain(time: f64) -> f64 {
    let phase = time.rem_euclid(BEAT_SECONDS);
    let pulse = if phase < 0.012 {
        smooth(phase / 0.012)
    } else {
        1.0 - smooth((phase - 0.012) / 0.20)
    };
    1.0 - 0.16 * pulse
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
        for chord in 0..HARMONIES.len() {
            let start_beat = cycle as f64 * 24.0 + CHORD_START_BEATS[chord];
            for (midi, level, pan) in [
                (BED_NOTES[chord], 0.0045, -0.08),
                (LOW_FIFTHS[chord], 0.0025, 0.08),
            ] {
                notes.push(Note {
                    start: start_beat * BEAT_SECONDS,
                    length: CHORD_LENGTH_BEATS[chord] * BEAT_SECONDS,
                    midi: f64::from(midi),
                    gain: level,
                    pan,
                    voice: Voice::LowBed,
                });
            }
            // Quiet, clipped polysynth support makes the harmony audible
            // without restoring the sustained midrange chord stack.
            for offset in [0.0, 1.0] {
                let beat = start_beat + offset + 0.02;
                for (index, midi) in HARMONIES[chord][..3].iter().enumerate() {
                    notes.push(Note {
                        start: beat * BEAT_SECONDS,
                        length: 0.24,
                        midi: f64::from(*midi),
                        gain: 0.014,
                        pan: [-0.42, 0.12, 0.42][index],
                        voice: Voice::Chord,
                    });
                }
            }
        }
    }
    for phrase in 0..6 {
        let start_beat = phrase as f64 * GROOVE_BEATS;
        let start = start_beat * BEAT_SECONDS;
        // Eighth-note propulsion with occasional octave replies. The first
        // downbeat defines each new harmony; the bass then plays around it.
        for step in 0..16 {
            let beat = step as f64 * 0.5;
            let chord = harmonic_index(start_beat + beat);
            let phase = (start_beat + beat).rem_euclid(24.0);
            let available = CHORD_START_BEATS[chord] + CHORD_LENGTH_BEATS[chord] - phase;
            let octave = if step % 4 == 3 { 12 } else { 0 };
            let touch = if step % 2 == 0 { 0.0 } else { 0.003 };
            notes.push(Note {
                start: start + beat * BEAT_SECONDS + touch,
                length: 0.17_f64.min(available * BEAT_SECONDS - touch),
                midi: f64::from(BASS_NOTES[chord] + octave),
                gain: if step % 2 == 0 { 0.093 } else { 0.070 },
                pan: 0.0,
                voice: Voice::Bass,
            });
        }
        for (beat, velocity) in KICK_PATTERN {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS,
                length: 0.22,
                midi: 40.0,
                gain: velocity,
                pan: 0.0,
                voice: Voice::Kick,
            });
        }
        for beat in [1.0, 3.0, 5.0, 7.0] {
            notes.push(Note {
                start: start + beat * BEAT_SECONDS + SNARE_LAG_SECONDS,
                length: 0.22,
                midi: 40.0,
                gain: if beat < 4.0 { 0.085 } else { 0.094 },
                pan: 0.06,
                voice: Voice::Snare,
            });
        }
        for step in 0..16 {
            let offbeat = step % 2 == 1;
            notes.push(Note {
                start: start + step as f64 * 0.5 * BEAT_SECONDS + if offbeat { 0.003 } else { 0.0 },
                length: if offbeat { 0.085 } else { 0.052 },
                midi: 40.0,
                gain: if offbeat { 0.048 } else { 0.028 },
                pan: if offbeat { 0.20 } else { -0.20 },
                voice: Voice::Hat,
            });
        }
        let returning = phrase >= 3;
        let melody = if returning {
            RESPONSE_MELODY[phrase % 3]
        } else {
            MELODY[phrase % 3]
        };
        let accents = [0.91, 0.73, 1.0, 0.82, 0.88, 0.76, 0.92, 0.78];
        let touches = [-0.003, 0.006, 0.0, 0.008, -0.002, 0.005, 0.007, 0.003];
        for (index, original_beat) in MELODY_BEATS.into_iter().enumerate() {
            let beat = original_beat + if returning && index == 3 { 0.125 } else { 0.0 };
            notes.push(Note {
                start: start + beat * BEAT_SECONDS + touches[index],
                length: MELODY_LENGTHS[index],
                midi: f64::from(melody[index]),
                gain: 0.15 * accents[index],
                pan: [-0.10, 0.04, 0.13, -0.05, 0.10, -0.12, 0.03, -0.04][index],
                voice: Voice::Lead,
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
            Voice::LowBed => smooth(time / 0.45) * smooth(remaining / 0.45) * phase.sin(),
            Voice::Bass => {
                let envelope =
                    smooth(time / 0.006) * smooth(remaining / 0.035) * (-time * 5.0).exp();
                envelope
                    * (0.58 * phase.sin() + 0.42 * analog_tone(phase, 0.20 + time * 4.0, false))
            }
            Voice::Lead | Voice::Chord => {
                let velocity = (note.gain / 0.15).clamp(0.0, 1.0);
                let envelope = lead_envelope(time, note.length, velocity);
                let tone = analog_tone(phase, 0.16 + time * 2.0, true);
                envelope * tone
            }
            Voice::Kick => {
                let envelope =
                    smooth(time / 0.005) * smooth(remaining / 0.06) * (-time * 15.0).exp();
                let sweep =
                    std::f64::consts::TAU * (49.0 * time + 1.4 * (1.0 - (-time * 24.0).exp()));
                envelope * sweep.sin()
            }
            Voice::Snare | Voice::Hat => {
                let bits = (offset as u32)
                    .wrapping_mul(747_796_405)
                    .wrapping_add((start as u32).wrapping_mul(2_891_336_453));
                let noise = f64::from((bits ^ (bits >> 16)) & 0xffff) / 32767.5 - 1.0;
                filtered_noise += 0.48 * (noise - filtered_noise);
                slow_noise += 0.08 * (filtered_noise - slow_noise);
                if note.voice == Voice::Hat {
                    smooth(time / 0.002)
                        * smooth(remaining / 0.025)
                        * (-time * 36.0).exp()
                        * (filtered_noise - slow_noise)
                } else {
                    let body = (std::f64::consts::TAU * 182.0 * time).sin() * (-time * 25.0).exp();
                    let clap = [0.0, 0.009, 0.018]
                        .into_iter()
                        .map(|delay| {
                            if time >= delay {
                                (-(time - delay) * 95.0).exp()
                            } else {
                                0.0
                            }
                        })
                        .sum::<f64>();
                    smooth(time / 0.002)
                        * smooth(remaining / 0.06)
                        * (0.48 * body
                            + 1.2 * filtered_noise * (0.5 * (-time * 15.0).exp() + 0.22 * clap))
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
        } else if matches!(note.voice, Voice::Lead | Voice::Chord) {
            &mut lead
        } else {
            &mut background
        };
        add_note(target, note);
    }
    let taps = [(BEAT_SECONDS * 0.75, 0.14), (BEAT_SECONDS * 1.5, 0.065)]
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
                (((melodic + background[index][channel]) * duck + drums[index][channel]) * 1.35)
                    .tanh();
        }
        mix.push(stereo);
    }
    // Two gentle stages tame the top end while retaining the saw's character.
    // Warm the filter over a whole cycle so its boundary remains periodic.
    let alpha = 1.0 - (-std::f64::consts::TAU * 3_000.0 / f64::from(SAMPLE_RATE)).exp();
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
    let master = MASTER_PEAK / peak.max(f64::EPSILON);
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

/// Create a quiet 24-second stereo, signed 16-bit PCM WAV at 22.05 kHz.
///
/// The 120-BPM synthwave theme keeps six/two-beat chord changes, with a warm
/// analog-style hook, pulsing octave bass and a straight dance backbeat.
/// Its restrained master level leaves room for the welcome UI. Rendering is cached.
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
    fn dance_groove_has_quarter_kicks_and_two_and_four_backbeats() {
        let notes = score_notes();
        let phrase: Vec<_> = notes
            .iter()
            .filter(|note| note.start >= 0.0 && note.start < GROOVE_BEATS * BEAT_SECONDS)
            .collect();
        let kicks: Vec<_> = phrase
            .iter()
            .filter(|note| note.voice == Voice::Kick)
            .collect();
        assert_eq!(kicks.len(), 8);
        for (beat, note) in kicks.iter().enumerate() {
            assert!((note.start / BEAT_SECONDS - beat as f64).abs() < 1e-9);
        }
        let snares: Vec<_> = phrase
            .iter()
            .filter(|note| note.voice == Voice::Snare)
            .collect();
        assert_eq!(snares.len(), 4);
        for (note, beat) in snares.iter().zip([1.0, 3.0, 5.0, 7.0]) {
            assert!((note.start - beat * BEAT_SECONDS - SNARE_LAG_SECONDS).abs() < 1e-9);
        }
        let hats: Vec<_> = phrase
            .iter()
            .filter(|note| note.voice == Voice::Hat)
            .collect();
        assert_eq!(hats.len(), 16);
        assert!(hats[1].gain > hats[0].gain);
        assert!(hats.iter().all(|note| note.length < 0.1));
    }

    #[test]
    fn octave_bass_drives_the_beat_and_follows_six_two_changes() {
        let notes = score_notes();
        let bass: Vec<_> = notes
            .iter()
            .filter(|note| note.voice == Voice::Bass)
            .collect();
        assert_eq!(bass.len(), 96);
        assert!(bass.iter().any(|note| note.midi >= 55.0));
        for note in bass {
            let beat = note.start / BEAT_SECONDS;
            let chord = harmonic_index(beat);
            assert_eq!(note.midi as i32 % 12, BASS_NOTES[chord] % 12);
            let phase = beat.rem_euclid(24.0);
            let available = CHORD_START_BEATS[chord] + CHORD_LENGTH_BEATS[chord] - phase;
            assert!(note.length <= available * BEAT_SECONDS + 1e-9);
        }
        assert_eq!(CHORD_START_BEATS, [0.0, 6.0, 8.0, 14.0, 16.0, 22.0]);
        assert_eq!(CHORD_LENGTH_BEATS, [6.0, 2.0, 6.0, 2.0, 6.0, 2.0]);
        for cycle in 0..2 {
            for chord in 0..6 {
                let start = cycle as f64 * 24.0 + CHORD_START_BEATS[chord];
                assert!(notes.iter().any(|note| note.voice == Voice::Bass
                    && (note.start / BEAT_SECONDS - start).abs() < 1e-9
                    && note.midi == f64::from(BASS_NOTES[chord])));
            }
        }
        assert_eq!(LOW_FIFTHS[4] % 12, 0); // C is the diminished fifth over F#.
    }

    #[test]
    fn analog_hook_has_articulated_phrases_and_a_varied_return() {
        let notes = score_notes();
        let leads: Vec<_> = notes
            .iter()
            .filter(|note| note.voice == Voice::Lead)
            .collect();
        assert_eq!(leads.len(), 48);
        assert!(
            leads
                .iter()
                .all(|note| (0.25..=0.45).contains(&note.length))
        );
        assert!(leads.iter().all(|note| (60.0..=72.0).contains(&note.midi)));
        for pair in leads.windows(2) {
            assert!(pair[1].start > pair[0].start + pair[0].length);
        }
        assert_ne!(leads[3].midi, leads[27].midi);
        assert!(lead_envelope(0.25, 0.42, 1.0) < lead_envelope(0.02, 0.42, 1.0) * 0.3);
        assert_eq!(lead_envelope(0.42, 0.42, 1.0), 0.0);
        assert_eq!(lead_envelope(0.50, 0.42, 1.0), 0.0);
        assert!(lead_envelope(0.25, 0.42, 0.5) < lead_envelope(0.25, 0.42, 1.0));
        let beds: Vec<_> = notes
            .iter()
            .filter(|note| note.voice == Voice::LowBed)
            .collect();
        assert!(
            beds.iter()
                .all(|note| note.midi <= 43.0 && note.gain <= 0.0045)
        );
        assert!(
            notes
                .iter()
                .filter(|note| note.length > 0.45)
                .all(|note| note.voice == Voice::LowBed)
        );
    }

    #[test]
    fn sidechain_breathes_gently_with_the_steady_kick() {
        assert!((BEAT_SECONDS * 48.0 - LOOP_SECONDS).abs() < 1e-12);
        assert!((sidechain(0.012) - 0.84).abs() < 1e-12);
        for sample in 0..1000 {
            let time = sample as f64 * BEAT_SECONDS / 1000.0;
            assert!((0.84..=1.0).contains(&sidechain(time)));
            assert!((sidechain(time) - sidechain(time + BEAT_SECONDS)).abs() < 1e-12);
        }
        assert!((sidechain(LOOP_SECONDS - 1e-6) - sidechain(0.0)).abs() < 1e-8);
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
        assert_eq!(LOOP_SECONDS, 24.0);
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
    fn mix_is_quiet_and_keeps_the_foreground_above_the_sustained_bed() {
        let output = samples(&synthesize_score(1.0));
        let normalized: Vec<_> = output
            .iter()
            .map(|sample| f64::from(*sample) / f64::from(i16::MAX))
            .collect();
        let energy = |values: &[f64]| values.iter().map(|sample| sample * sample).sum::<f64>();
        let rms = (energy(&normalized) / normalized.len() as f64).sqrt();
        let peak = normalized
            .iter()
            .fold(0.0_f64, |peak, sample| peak.max(sample.abs()));
        assert!((0.04..0.12).contains(&rms), "RMS {rms}");
        assert!((0.44..0.46).contains(&peak));
        let differences: Vec<_> = normalized
            .iter()
            .zip(normalized.iter().skip(2))
            .map(|(a, b)| a - b)
            .collect();
        let difference_rms = (energy(&differences) / differences.len() as f64).sqrt();
        assert!(difference_rms / rms < 0.35); // Character without a harsh upper register.
        let mut lead = vec![[0.0_f64; 2]; FRAME_COUNT];
        let mut bed = vec![[0.0_f64; 2]; FRAME_COUNT];
        for note in score_notes() {
            if note.voice == Voice::Lead {
                add_note(&mut lead, note);
            } else if note.voice == Voice::LowBed {
                add_note(&mut bed, note);
            }
        }
        let stem_energy = |stem: &[[f64; 2]]| stem.iter().flatten().map(|v| v * v).sum::<f64>();
        assert!((stem_energy(&lead) / stem_energy(&bed)).sqrt() > 4.0);
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
            assert!(seam.abs() <= before.abs().max(after.abs()) + 300);
            assert!(
                (seam - before).abs() < 300,
                "before seam slope {before} -> {seam}"
            );
            assert!(
                (after - seam).abs() < 300,
                "after seam slope {seam} -> {after}"
            );
        }
    }
}
