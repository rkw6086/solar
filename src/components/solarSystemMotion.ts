export interface OrbitTrack<T> {
  orbit: T;
  x: number;
  y: number;
  phase: number;
  period: number;
}

// Stable starting phases keep the scene familiar each time it is opened.
function startingPhase(id: string, index: number, count: number) {
  let hash = 2166136261;
  for (const character of id) {
    hash = Math.imul(hash ^ character.charCodeAt(0), 16777619);
  }
  const spacing = (Math.PI * 2) / count;
  // Distribute the roster around the whole star first. Similar IDs can hash
  // almost identically, so hashing alone must not decide their separation.
  const jitter = (((hash >>> 0) / 4294967296) - 0.5) * spacing * 0.2;
  const phase = -Math.PI / 2 + index * spacing + jitter;
  return (phase + Math.PI * 2) % (Math.PI * 2);
}

export function orbitTracks<T extends { id: string }>(orbits: T[]): OrbitTrack<T>[] {
  const roster = [...orbits].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return roster.map((orbit, index) => {
    const distance = roster.length === 1 ? 0.32 : 0.22 + (index / (roster.length - 1)) * 0.18;
    return {
      orbit,
      x: distance,
      y: distance * 0.875,
      phase: startingPhase(orbit.id, index, roster.length),
      // Every period is an integer multiple of the inner orbit's period.
      period: 48_000 * (index + 1),
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
