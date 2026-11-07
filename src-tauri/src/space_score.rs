//! A bright, repeating space-pop score made from MIDI pitches and synthesis.
//!
//! Notes, releases and delay taps wrap around the same 38.4-second timeline.
//! No recordings or network assets are used. The major I–V–vi–IV pop schema is
//! discussed in Open Music Theory:
//! https://viva.pressbooks.pub/openmusictheory/chapter/4-chord-schemas/

use std::sync::LazyLock;

const SAMPLE_RATE: u32 = 22_050;
const CHANNELS: u16 = 2;
const LOOP_MILLISECONDS: u32 = 38_400;
const LOOP_SECONDS: f64 = 38.4;
const BEAT_SECONDS: f64 = 0.6; // 100 BPM, four 4/4 bars per harmony.
const FRAME_COUNT: usize = SAMPLE_RATE as usize * LOOP_MILLISECONDS as usize / 1000;

#[derive(Clone, Copy)]
enum Voice {
    Pad,
    Bass,
    Bell,
    Pluck,
    Kick,
    Hat,
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

fn add_note(buffer: &mut [[f64; 2]], note: Note) {
    let frequency = 440.0 * 2.0_f64.powf((note.midi - 69.0) / 12.0);
    let start = (note.start.rem_euclid(LOOP_SECONDS) * f64::from(SAMPLE_RATE)).round() as usize;
    let frames = (note.length * f64::from(SAMPLE_RATE)).round() as usize;
    let left = ((1.0 - note.pan.clamp(-1.0, 1.0)) / 2.0).sqrt();
    let right = ((1.0 + note.pan.clamp(-1.0, 1.0)) / 2.0).sqrt();
    for offset in 0..frames {
        let time = offset as f64 / f64::from(SAMPLE_RATE);
        let phase = std::f64::consts::TAU * frequency * time;
        let remaining = note.length - time;
        let sample = match note.voice {
            Voice::Pad => {
                let envelope = smooth(time / 1.5) * smooth(remaining / 1.8);
                let breath = 0.94 + 0.06 * (std::f64::consts::TAU * time / 6.0).sin();
                let tone = 0.64 * phase.sin()
                    + 0.17 * (phase * 0.9986).sin()
                    + 0.17 * (phase * 1.0014).sin()
                    + 0.045 * (phase * 2.0).sin();
                envelope * breath * tone
            }
            Voice::Bass => {
                let envelope =
                    smooth(time / 0.015) * smooth(remaining / 0.18) * (-time * 0.9).exp();
                envelope * (0.92 * phase.sin() + 0.08 * (phase * 2.0).sin())
            }
            Voice::Bell => {
                let envelope =
                    smooth(time / 0.025) * smooth(remaining / 0.6) * (-time * 1.65).exp();
                envelope
                    * (0.83 * phase.sin() + 0.12 * (phase * 2.0).sin() + 0.05 * (phase * 3.0).sin())
            }
            Voice::Pluck => {
                let envelope =
                    smooth(time / 0.008) * smooth(remaining / 0.28) * (-time * 3.5).exp();
                envelope
                    * (0.77 * phase.sin() + 0.18 * (phase * 2.0).sin() + 0.05 * (phase * 3.0).sin())
            }
            Voice::Kick => {
                let envelope =
                    smooth(time / 0.006) * smooth(remaining / 0.08) * (-time * 18.0).exp();
                let sweep =
                    std::f64::consts::TAU * (58.0 * time + 4.0 * (1.0 - (-time * 22.0).exp()));
                envelope * sweep.sin()
            }
            Voice::Hat => {
                let bits = (offset as u32)
                    .wrapping_mul(747_796_405)
                    .wrapping_add(2_891_336_453);
                let noise = f64::from((bits ^ (bits >> 16)) & 0xffff) / 32767.5 - 1.0;
                smooth(time / 0.002) * smooth(remaining / 0.012) * (-time * 35.0).exp() * noise
            }
        } * note.gain;
        let frame = &mut buffer[(start + offset) % FRAME_COUNT];
        frame[0] += sample * left;
        frame[1] += sample * right;
    }
}

fn render_score() -> Vec<i16> {
    let mut dry = vec![[0.0_f64; 2]; FRAME_COUNT];
    let chords = [
        [48, 55, 60, 62, 64], // Cadd9: I.
        [47, 50, 55, 59, 62], // G: V.
        [45, 52, 55, 60, 64], // Am7: vi.
        [48, 53, 57, 60, 64], // Fmaj7: IV.
    ];
    let bass = [48, 43, 45, 41];
    let pans = [-0.65, 0.45, -0.2, 0.7, 0.0];
    let arpeggio = [0, 2, 4, 3, 2, 4, 1, 3];
    let melody = [
        [76, 79, 81, 79, 76, 74, 72],
        [74, 79, 76, 74, 72, 74, 79],
        [76, 81, 79, 76, 74, 76, 72],
        [76, 79, 81, 79, 76, 74, 72],
    ];
    let melody_beats = [0.0, 3.0, 5.0, 7.5, 9.0, 11.0, 14.0];
    for (chord_index, chord) in chords.iter().enumerate() {
        let bar_start = chord_index as f64 * 16.0 * BEAT_SECONDS;
        for (voice_index, midi) in chord.iter().enumerate() {
            add_note(
                &mut dry,
                Note {
                    start: bar_start - 0.6,
                    length: 10.8,
                    midi: f64::from(*midi),
                    gain: 0.024,
                    pan: pans[voice_index],
                    voice: Voice::Pad,
                },
            );
        }
        for beat in (0..16).step_by(2) {
            let start = bar_start + beat as f64 * BEAT_SECONDS;
            add_note(
                &mut dry,
                Note {
                    start,
                    length: 0.52,
                    midi: f64::from(bass[chord_index]),
                    gain: 0.058,
                    pan: 0.0,
                    voice: Voice::Bass,
                },
            );
            add_note(
                &mut dry,
                Note {
                    start,
                    length: 0.24,
                    midi: 36.0,
                    gain: if beat % 4 == 0 { 0.028 } else { 0.018 },
                    pan: 0.0,
                    voice: Voice::Kick,
                },
            );
        }
        for beat in 0..16 {
            add_note(
                &mut dry,
                Note {
                    start: bar_start + (beat as f64 + 0.5) * BEAT_SECONDS,
                    length: 0.065,
                    midi: 90.0,
                    gain: 0.009,
                    pan: 0.1,
                    voice: Voice::Hat,
                },
            );
        }
        for eighth in 0..32 {
            let degree = arpeggio[eighth % arpeggio.len()];
            add_note(
                &mut dry,
                Note {
                    start: bar_start + eighth as f64 * BEAT_SECONDS / 2.0,
                    length: 1.35,
                    midi: f64::from(chord[degree]) + 12.0,
                    gain: if eighth % 4 == 0 { 0.025 } else { 0.017 },
                    pan: if eighth % 2 == 0 { -0.45 } else { 0.45 },
                    voice: Voice::Pluck,
                },
            );
        }
        for (index, midi) in melody[chord_index].iter().enumerate() {
            add_note(
                &mut dry,
                Note {
                    start: bar_start + melody_beats[index] * BEAT_SECONDS + 0.025,
                    length: 2.1,
                    midi: f64::from(*midi),
                    gain: 0.032,
                    pan: 0.12,
                    voice: Voice::Bell,
                },
            );
        }
    }
    let taps = [
        (0.3, 0.18),
        (0.6, 0.12),
        (0.9, 0.075),
        (1.2, 0.045),
        (1.8, 0.025),
    ];
    let taps =
        taps.map(|(delay, amount)| ((delay * f64::from(SAMPLE_RATE)).round() as usize, amount));
    let mut rendered = Vec::with_capacity(FRAME_COUNT * usize::from(CHANNELS));
    for (index, frame) in dry.iter().enumerate() {
        for (channel, direct) in frame.iter().enumerate() {
            let mut sample = *direct;
            for (tap_index, (delay, amount)) in taps.iter().enumerate() {
                let delayed = (index + FRAME_COUNT - delay) % FRAME_COUNT;
                let source_channel = if tap_index % 2 == 0 {
                    1 - channel
                } else {
                    channel
                };
                sample += dry[delayed][source_channel] * amount;
            }
            let sample = 0.9 * (sample * 1.6).tanh();
            rendered.push(sample);
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

/// Create a 38.4-second stereo, signed 16-bit PCM WAV at 22.05 kHz.
///
/// The 100-BPM I–V–vi–IV loop uses Cadd9, G, Am7 and Fmaj7. A C-major
/// pentatonic melody, light arpeggio, short round bass and restrained pulse
/// keep the space textures warm and uplifting. Rendering is cached once.
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
            assert!(seam.abs() < 1200, "seam step {seam}");
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
