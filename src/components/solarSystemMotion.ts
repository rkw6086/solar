export interface OrbitTrack<T> {
  orbit: T;
  x: number;
  y: number;
  phase: number;
  period: number;
}

export const ORBIT_PERIOD = 120_000;

// Six on the inner lane, twelve on the next, then eighteen. Sharing a lane's
// speed preserves spacing instead of letting the planets slowly collide.
export function orbitTracks<T extends { id: string }>(orbits: T[]): OrbitTrack<T>[] {
  const roster = [...orbits].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  const tracks: OrbitTrack<T>[] = [];
  let lanes = 0;
  let remaining = roster.length;
  while (remaining > 0) { lanes += 1; remaining -= 6 * lanes; }
  // Match native lane proportions. Reserve room for every lane instead of
  // stacking the third and later lanes against the outer edge.
  const laneWidth = lanes > 2 ? 0.4 / lanes : 0.2;
  let offset = 0;
  let ring = 0;
  while (offset < roster.length) {
    const count = Math.min(6 * (ring + 1), roster.length - offset);
    const distance = laneWidth * (ring + 1);
    for (let index = 0; index < count; index += 1) {
      tracks.push({
        orbit: roster[offset + index],
        x: distance,
        y: distance,
        phase: -Math.PI / 2 + index * Math.PI * 2 / count + (ring % 2 ? Math.PI / count : 0),
        period: ORBIT_PERIOD * 2 ** ring,
      });
    }
    offset += count;
    ring += 1;
  }
  return tracks;
}

export function orbitPosition(track: OrbitTrack<unknown>, elapsed: number, width: number, height: number) {
  const angle = track.phase + (elapsed / track.period) * Math.PI * 2;
  return {
    x: Math.cos(angle) * width * track.x,
    y: Math.sin(angle) * height * track.y,
  };
}
