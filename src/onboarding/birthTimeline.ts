export type BirthSoundCue = "ripple" | "formation" | "wink";

export const BIRTH_TIMING = {
  opening: 10_400,
  burst: 6_800,
  zoom: 7_700,
  solar: 8_100,
  wink: 9_500,
  winkDuration: 360,
  reducedOpening: 1_100,
  reducedFormation: 160,
} as const;

export const RIPPLE_COUNT = 18;

export function rippleTiming(index: number) {
  const start = 650 + index * (330 - index * 5);
  const duration = 2_250 - index * 22;
  return { start, duration, arrival: start + duration };
}

export interface TimedBirthCue {
  cue: BirthSoundCue;
  at: number;
  index?: number;
}

const ripples: TimedBirthCue[] = Array.from({ length: RIPPLE_COUNT }, (_, index) => ({
  cue: "ripple",
  index,
  at: rippleTiming(index).arrival,
}));

const cues: TimedBirthCue[] = [
  ...ripples,
  { cue: "formation", at: BIRTH_TIMING.burst },
  { cue: "wink", at: BIRTH_TIMING.wink },
];

/** Emit only cues the visible animation has just reached, never a stale audio queue. */
export function birthCuesBetween(previous: number, elapsed: number, reducedMotion: boolean): TimedBirthCue[] {
  if (elapsed <= previous) return [];
  const timeline: TimedBirthCue[] = reducedMotion
    ? [{ cue: "formation", at: BIRTH_TIMING.reducedFormation }]
    : cues;
  return timeline.filter(({ at }) => at > previous && at <= elapsed && elapsed - at <= 120);
}

/** Close, hold briefly, then reopen the right eye. Reduced motion never calls this. */
export function winkAmount(elapsed: number): number {
  const phase = (elapsed - BIRTH_TIMING.wink) / BIRTH_TIMING.winkDuration;
  if (phase <= 0 || phase >= 1) return 0;
  const smooth = (value: number) => value * value * (3 - 2 * value);
  if (phase < 0.3) return smooth(phase / 0.3);
  if (phase < 0.6) return 1;
  return 1 - smooth((phase - 0.6) / 0.4);
}
