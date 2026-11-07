//! A bright, repeating melodic EDM score made from MIDI pitches and synthesis.
//!
//! The 128-BPM, 16-bar loop stays in C major. Its four-on-the-floor beat,
//! offbeat bass, syncopated chord stabs and bright lead make the welcome lively.
//! Notes, releases and stereo delay wrap around the same 30-second timeline.
//! No recordings or network assets are used. The I–V–vi–IV harmony is discussed
//! in Open Music Theory:
//! https://viva.pressbooks.pub/openmusictheory/chapter/4-chord-schemas/

use std::sync::LazyLock;

const SAMPLE_RATE: u32 = 22_050;
const CHANNELS: u16 = 2;
const BPM: f64 = 128.0;
const LOOP_MILLISECONDS: u32 = 30_000;
const LOOP_SECONDS: f64 = 30.0;
const BEAT_SECONDS: f64 = 60.0 / BPM;
const FRAME_COUNT: usize = SAMPLE_RATE as usize * LOOP_MILLISECONDS as usize / 1000;

#[derive(Clone, Copy)]
enum Voice {
    Pad,
    Bass,
    Lead,
    Stab,
    Kick,
    Clap,
    ClosedHat,
    OpenHat,
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

/// A polynomial correction removes a sawtooth's discontinuity before it aliases.
fn poly_blep(phase: f64, increment: f64) -> f64 {
    if phase < increment {
        let x = phase / increment;
        2.0 * x - x * x - 1.0
    } else if phase > 1.0 - increment {
        let x = (phase - 1.0) / increment;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}

fn saw(phase: f64, increment: f64) -> f64 {
    2.0 * phase - 1.0 - poly_blep(phase, increment)
}

/// Fast attack and a musical recovery, continuous across each beat/loop edge.
fn sidechain(time: f64) -> f64 {
    let phase = (time / BEAT_SECONDS).rem_euclid(1.0);
    if phase < 0.035 {
        1.0 - 0.8 * smooth(phase / 0.035)
    } else {
        0.2 + 0.8 * smooth((phase - 0.035) / 0.58)
    }
}

fn add_note(buffer: &mut [[f64; 2]], note: Note) {
    let frequency = 440.0 * 2.0_f64.powf((note.midi - 69.0) / 12.0);
    let start = (note.start.rem_euclid(LOOP_SECONDS) * f64::from(SAMPLE_RATE)).round() as usize;
    let frames = (note.length * f64::from(SAMPLE_RATE)).round() as usize;
    let left = ((1.0 - note.pan.clamp(-1.0, 1.0)) / 2.0).sqrt();
    let right = ((1.0 + note.pan.clamp(-1.0, 1.0)) / 2.0).sqrt();
    let increments = [
        frequency / f64::from(SAMPLE_RATE),
        frequency * 0.996 / f64::from(SAMPLE_RATE),
        frequency * 1.004 / f64::from(SAMPLE_RATE),
    ];
    let mut phases = [0.0, 0.29, 0.67];
    let mut filtered = 0.0;
    let mut previous_noise = 0.0;
    for offset in 0..frames {
        let time = offset as f64 / f64::from(SAMPLE_RATE);
        let phase = std::f64::consts::TAU * frequency * time;
        let remaining = note.length - time;
        let bits = (offset as u32)
            .wrapping_mul(747_796_405)
            .wrapping_add(2_891_336_453);
        let noise = f64::from((bits ^ (bits >> 16)) & 0xffff) / 32767.5 - 1.0;
        let bright_noise = (noise - previous_noise) * 0.55;
        previous_noise = noise;
        let supersaw = (saw(phases[0], increments[0])
            + saw(phases[1], increments[1])
            + saw(phases[2], increments[2]))
            / 3.0;
        for (phase, increment) in phases.iter_mut().zip(increments) {
            *phase = (*phase + increment).fract();
        }
        let sample = match note.voice {
            Voice::Pad => {
                filtered += 0.15 * (supersaw - filtered);
                let envelope = smooth(time / 0.2) * smooth(remaining / 0.45);
                envelope * (filtered * 0.65 + phase.sin() * 0.35)
            }
            Voice::Bass => {
                filtered += 0.09 * (supersaw - filtered);
                let envelope =
                    smooth(time / 0.004) * smooth(remaining / 0.05) * (-time * 3.0).exp();
                envelope * (phase.sin() * 0.68 + filtered * 0.52)
            }
            Voice::Lead => {
                filtered += 0.48 * (supersaw - filtered);
                let envelope =
                    smooth(time / 0.008) * smooth(remaining / 0.045) * (-time * 0.65).exp();
                envelope * (filtered * 0.82 + phase.sin() * 0.3)
            }
            Voice::Stab => {
                filtered += 0.33 * (supersaw - filtered);
                let envelope =
                    smooth(time / 0.004) * smooth(remaining / 0.035) * (-time * 5.5).exp();
                envelope * (filtered * 0.85 + phase.sin() * 0.22)
            }
            Voice::Kick => {
                let envelope =
                    smooth(time / 0.0015) * smooth(remaining / 0.06) * (-time * 10.0).exp();
                let sweep =
                    std::f64::consts::TAU * (49.0 * time + 3.8 * (1.0 - (-time * 38.0).exp()));
                envelope * (sweep.sin() + bright_noise * 0.055 * (-time * 180.0).exp())
            }
            Voice::Clap => {
                let bursts = [0.0, 0.012, 0.024]
                    .into_iter()
                    .map(|delay| {
                        let t = time - delay;
                        if t < 0.0 {
                            0.0
                        } else {
                            smooth(t / 0.001) * (-t * 75.0).exp()
                        }
                    })
                    .sum::<f64>();
                let body = smooth(time / 0.001) * (-time * 16.0).exp() * 0.22;
                smooth(remaining / 0.045) * bright_noise * (bursts * 0.6 + body)
            }
            Voice::ClosedHat => {
                let envelope =
                    smooth(time / 0.0015) * smooth(remaining / 0.012) * (-time * 60.0).exp();
                filtered += 0.72 * (bright_noise - filtered);
                envelope * filtered
            }
            Voice::OpenHat => {
                let envelope =
                    smooth(time / 0.002) * smooth(remaining / 0.04) * (-time * 14.0).exp();
                filtered += 0.65 * (bright_noise - filtered);
                envelope * filtered
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
    let chords = [
        [60, 64, 67, 74], // Cadd9: I.
        [59, 62, 67, 69], // Gadd9: V.
        [57, 60, 64, 67], // Am7: vi.
        [57, 60, 64, 65], // Fmaj7: IV.
    ];
    let bass = [36, 43, 45, 41];
    let lead = [
        [79, 79, 81, 79, 76, 74, 76, 79],
        [79, 74, 76, 79, 81, 79, 74, 74],
        [81, 81, 79, 76, 79, 76, 74, 76],
        [79, 81, 79, 76, 74, 76, 72, 72],
    ];
    let melody_beats = [0.0, 1.5, 2.0, 2.75, 3.5, 4.5, 6.0, 7.25];
    for (chord_index, chord) in chords.iter().enumerate() {
        let start = chord_index as f64 * 16.0 * BEAT_SECONDS;
        for (voice, midi) in chord.iter().enumerate() {
            add_note(
                &mut music,
                Note {
                    start: start - 0.2,
                    length: 7.9,
                    midi: f64::from(*midi) - 12.0,
                    gain: 0.055,
                    pan: [-0.6, 0.5, -0.2, 0.7][voice],
                    voice: Voice::Pad,
                },
            );
        }
        for beat in 0..16 {
            let time = start + f64::from(beat) * BEAT_SECONDS;
            // Four-on-the-floor begins on the first sample, not after a long pad introduction.
            add_note(
                &mut drums,
                Note {
                    start: time,
                    length: 0.34,
                    midi: 36.0,
                    gain: 0.35,
                    pan: 0.0,
                    voice: Voice::Kick,
                },
            );
            if beat % 2 == 1 {
                add_note(
                    &mut drums,
                    Note {
                        start: time,
                        length: 0.23,
                        midi: 60.0,
                        gain: 0.16,
                        pan: 0.0,
                        voice: Voice::Clap,
                    },
                );
            }
            add_note(
                &mut drums,
                Note {
                    start: time + BEAT_SECONDS * 0.5,
                    length: 0.20,
                    midi: 90.0,
                    gain: 0.105,
                    pan: 0.18,
                    voice: Voice::OpenHat,
                },
            );
            add_note(
                &mut music,
                Note {
                    start: time + BEAT_SECONDS * 0.5,
                    length: BEAT_SECONDS * 0.45,
                    midi: f64::from(bass[chord_index]),
                    gain: 0.19,
                    pan: 0.0,
                    voice: Voice::Bass,
                },
            );
            for sixteenth in [0.0, 0.25, 0.75] {
                add_note(
                    &mut drums,
                    Note {
                        start: time + BEAT_SECONDS * sixteenth,
                        length: 0.055,
                        midi: 90.0,
                        gain: if sixteenth == 0.0 { 0.055 } else { 0.028 },
                        pan: -0.25,
                        voice: Voice::ClosedHat,
                    },
                );
            }
        }
        for bar in 0..4 {
            for (position, beat) in [0.5, 1.5, 2.75, 3.5].into_iter().enumerate() {
                for (voice, midi) in chord.iter().enumerate() {
                    add_note(
                        &mut music,
                        Note {
                            start: start + (f64::from(bar) * 4.0 + beat) * BEAT_SECONDS,
                            length: if position == 2 { 0.15 } else { 0.24 },
                            midi: f64::from(*midi),
                            gain: 0.095,
                            pan: [-0.35, 0.35, -0.6, 0.6][voice],
                            voice: Voice::Stab,
                        },
                    );
                }
            }
        }
        for repeat in 0..2 {
            for (index, midi) in lead[chord_index].iter().enumerate() {
                let long = matches!(index, 0 | 5 | 7);
                add_note(
                    &mut music,
                    Note {
                        start: start
                            + (f64::from(repeat) * 8.0 + melody_beats[index]) * BEAT_SECONDS,
                        length: if long { 0.42 } else { 0.26 },
                        midi: f64::from(*midi),
                        gain: if long { 0.18 } else { 0.15 },
                        pan: 0.05,
                        voice: Voice::Lead,
                    },
                );
            }
        }
    }
    // Only the melodic bus receives a restrained dotted-eighth stereo echo;
    // the kick and clap stay dry and tight rather than washing out the rhythm.
    let taps = [
        (BEAT_SECONDS * 0.75, 0.17),
        (BEAT_SECONDS * 1.5, 0.09),
        (BEAT_SECONDS * 2.25, 0.045),
    ];
    let taps =
        taps.map(|(delay, amount)| ((delay * f64::from(SAMPLE_RATE)).round() as usize, amount));
    let mut rendered = Vec::with_capacity(FRAME_COUNT * usize::from(CHANNELS));
    for (index, frame) in music.iter().enumerate() {
        let duck = sidechain(index as f64 / f64::from(SAMPLE_RATE));
        for (channel, direct) in frame.iter().enumerate() {
            let mut melodic = *direct;
            for (tap, (delay, amount)) in taps.iter().enumerate() {
                melodic += music[(index + FRAME_COUNT - delay) % FRAME_COUNT]
                    [if tap % 2 == 0 { 1 - channel } else { channel }]
                    * amount;
            }
            let mixed = melodic * duck + drums[index][channel];
            rendered.push((mixed * 5.0).tanh());
        }
    }
    let peak = rendered
        .iter()
        .fold(0.0_f64, |peak, sample| peak.max(sample.abs()));
    let master = 0.82 / peak.max(f64::EPSILON);
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

/// Create a 30-second stereo, signed 16-bit PCM WAV at 22.05 kHz.
///
/// The 128-BPM I–V–vi–IV loop uses Cadd9, Gadd9, Am7 and Fmaj7. A C-major
/// lead, pumping chord stabs, offbeat bass and four-on-the-floor rhythm
/// keep the electronic arrangement upbeat and welcoming. Rendering is cached once.
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
    fn dance_pulse_is_periodic_and_recovers_between_kicks() {
        assert!((BEAT_SECONDS * 64.0 - LOOP_SECONDS).abs() < 1e-12);
        assert!((sidechain(0.0) - 1.0).abs() < 1e-12);
        assert!(sidechain(BEAT_SECONDS * 0.035) <= 0.201);
        assert!(sidechain(BEAT_SECONDS * 0.8) >= 0.999);
        assert!((sidechain(LOOP_SECONDS - 1e-6) - sidechain(0.0)).abs() < 1e-8);
        for part in [0.0, 0.015, 0.035, 0.3, 0.8] {
            assert!(
                (sidechain(part * BEAT_SECONDS) - sidechain((part + 4.0) * BEAT_SECONDS)).abs()
                    < 1e-12
            );
        }
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
