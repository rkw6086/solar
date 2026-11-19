export interface OrbitTrack<T> {
  orbit: T;
  x: number;
  y: number;
  phase: number;
  period: number;
}

export const ORBIT_PERIOD = 120_000;

// Each Sprite owns a separate path. Golden-angle phases spread the team around
// the star while equal-area radial spacing leaves room for the inner paths.
// A shared 1:1 resonance preserves that spacing throughout a complete orbit;
// neighboring paths never overtake each other on compact screens.
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));
const INNER_PATH = .22;
const OUTER_PATH = .42;

export function orbitTracks<T extends { id: string }>(orbits: T[]): OrbitTrack<T>[] {
  const roster = [...orbits].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return roster.map((orbit, index) => {
    const fraction = roster.length > 1 ? index / (roster.length - 1) : 0;
    const radius = Math.sqrt(INNER_PATH ** 2 + (OUTER_PATH ** 2 - INNER_PATH ** 2) * fraction);
    return {
      orbit,
      x: radius,
      y: radius,
      phase: -Math.PI / 2 + index * GOLDEN_ANGLE,
      period: ORBIT_PERIOD,
    };
  });
}

export function orbitPosition(track: OrbitTrack<unknown>, elapsed: number, width: number, height: number) {
  const angle = track.phase + (elapsed / track.period) * Math.PI * 2;
  return {
    x: Math.cos(angle) * width * track.x,
    y: Math.sin(angle) * height * track.y,
  };
}
