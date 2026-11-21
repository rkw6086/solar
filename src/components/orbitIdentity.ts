const names = [
  "Nova", "Aster", "Lyra", "Vega", "Cosmo", "Sora", "Atlas", "Lumi",
  "Solstice", "Comet", "Orion", "Juniper", "Elara", "Pip", "Andromeda", "Cleo",
  "Mira", "Halo", "Celeste", "Astra", "Io", "Clover", "Polaris", "Echo",
  "Zephyr", "Indigo", "Arlo", "Nimbus", "Stella", "Quill", "Aurora", "Luna",
  "Cinder", "Tala", "Sunny", "Pixel", "Phoebe", "Apollo", "Scout", "Opal",
] as const;
const temperaments = [
  "Warm and encouraging", "Calm and thoughtful", "Bright and curious", "Playful and inventive",
  "Patient and practical", "Candid and kind", "Quietly enthusiastic", "Cheerful and resourceful",
];
const approaches = [
  "asks thoughtful questions and explains things clearly",
  "notices small details without losing sight of the goal",
  "turns complicated work into manageable next steps",
  "enjoys exploring possibilities, then checks the evidence",
  "keeps a cool head and brings gentle humor to difficult tasks",
  "is organized, reliable, and comfortable admitting uncertainty",
  "celebrates progress while giving honest, useful feedback",
  "is concise by default and happy to explain more when needed",
];
const quirks = [
  "A little dry wit, never at anyone’s expense.",
  "Occasional space puns, used sparingly.",
  "Friendly enthusiasm without overdoing it.",
  "A reassuring presence with a touch of mischief.",
  "An explorer’s curiosity and a teammate’s consideration.",
  "A fondness for neat solutions and small discoveries.",
];

function pick<T>(items: readonly T[], random: () => number): T {
  return items[Math.min(items.length - 1, Math.max(0, Math.floor(random() * items.length)))];
}

/** Offer a fresh suggestion without taking away the user's editable fields. */
export function generateOrbitName(existing: readonly string[] = [], random = Math.random): string {
  const used = new Set(existing.map((name) => name.trim().toLocaleLowerCase()));
  const available = names.filter((name) => !used.has(name.toLocaleLowerCase()));
  if (available.length) return pick(available, random);
  const base = pick(names, random);
  let suffix = 2;
  while (used.has(`${base} ${suffix}`.toLocaleLowerCase())) suffix += 1;
  return `${base} ${suffix}`;
}

export function generateOrbitPersonality(random = Math.random): string {
  return `${pick(temperaments, random)}; ${pick(approaches, random)}. ${pick(quirks, random)}`;
}

export const eyeStyles = ["bright", "happy", "curious", "relaxed", "sparkle", "round", "wink", "wide", "sleepy", "focused", "lashes", "glasses", "star", "crescent"] as const;
export const smileStyles = ["smile", "grin", "cheeky", "beam", "soft", "smirk", "open", "delighted", "cat", "tooth", "surprised"] as const;
export const planetStyles = ["rocky", "ocean", "cloudy", "banded", "cratered", "icy", "lava", "crystal", "marbled"] as const;
export type EyeStyle = typeof eyeStyles[number];
export type SmileStyle = typeof smileStyles[number];
export type PlanetStyle = typeof planetStyles[number];
export type RingStyle = "none" | "flat" | "tilted" | "double";
export interface OrbitAppearance {
  eyes: EyeStyle;
  smile: SmileStyle;
  surface: PlanetStyle;
  rings: RingStyle;
  freckles: boolean;
  blush: boolean;
  variation: number;
}

function appearanceHash(value: string): number {
  let hash = 2166136261;
  for (const character of value) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  // Mix the high and low bits so nearby Sprite IDs do not create similar worlds.
  hash ^= hash >>> 16;
  hash = Math.imul(hash, 0x7feb352d) >>> 0;
  hash ^= hash >>> 15;
  return hash >>> 0;
}

/** Stable across rerenders, tabs, restarts, and the desktop companion. */
export function orbitAppearance(identity: string, color = ""): OrbitAppearance {
  const key = `${identity}:${color}`;
  const markings = appearanceHash(`${key}:markings`);
  const rings: RingStyle[] = ["none", "none", "none", "none", "flat", "tilted", "double"];
  return {
    eyes: eyeStyles[appearanceHash(`${key}:eyes`) % eyeStyles.length],
    smile: smileStyles[appearanceHash(`${key}:smile`) % smileStyles.length],
    surface: planetStyles[appearanceHash(`${key}:surface`) % planetStyles.length],
    rings: rings[appearanceHash(`${key}:rings`) % rings.length],
    freckles: (markings & 1) !== 0,
    blush: (markings & 2) !== 0,
    variation: appearanceHash(`${key}:variation`) % 8,
  };
}

/** A fresh face without renaming the Sprite or changing its workspace identity. */
export function generateOrbitAvatarSeed(identity: string, color: string, newSeed = () => crypto.randomUUID()): string {
  const current = orbitAppearance(identity, color);
  const seed = newSeed();
  for (let attempt = 0; attempt < 128; attempt += 1) {
    const candidate = attempt === 0 ? seed : `${seed}-${attempt}`;
    const appearance = orbitAppearance(candidate, color);
    const freshFace = appearance.eyes !== current.eyes || appearance.smile !== current.smile;
    const freshPlanet = appearance.surface !== current.surface || appearance.rings !== current.rings;
    if (candidate !== identity && freshFace && freshPlanet) return candidate;
  }
  throw new Error("Could not create a new avatar. Please try again.");
}
