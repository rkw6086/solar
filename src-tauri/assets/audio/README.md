# Solar welcome theme

`solar-welcome.wav` is the accepted Take 1 melodic-trance/synthwave audition, with softer upper percussion and an 8% tempo reduction that preserves pitch. It plays only during the welcome/setup experience. The app embeds this file; playback makes no network calls.

The original instrumental was generated for Solar with ElevenLabs Music v2 on November 7, 2026 (generation `KEhoETUt7HU97hypgKEO`). Its direction was a warm, minor-key theme with a prominent rolling bass line, an expressive melodic hook, and restrained drums. No existing song or artist recording was used as a reference.

Processing before the PCM export:

- Normalize the audition to a target of -23 LUFS, with a -3 dB true-peak ceiling.
- Apply a gentle -1 dB bell at 3.2 kHz and a -2.5 dB high shelf at 5.5 kHz to soften bright percussion.
- Fade the first 150 ms and final 600 ms of the original 30-second audition.
- Slow playback to 92% with pitch-preserving time stretching.
- Export stereo, 44.1 kHz, 16-bit PCM WAV with a standard 44-byte header.

The final file contains 1,437,397 frames (32.594 seconds). The quiet beginning and ending avoid a click as native playback repeats. App-level volume and welcome transition fades are applied separately, with a default gain of 0.12. Existing saved volume choices are preserved.

SHA-256: `516f2e0702ca148a7a26cc3633e196d6bbc89d28703d50997b958cf0b8174317`
