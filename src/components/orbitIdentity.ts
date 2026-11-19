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

export type EyeStyle = "bright" | "happy" | "curious" | "relaxed" | "sparkle" | "round";
export type SmileStyle = "smile" | "grin" | "cheeky" | "beam" | "soft";

/** Stable across rerenders, tabs, restarts, and the desktop companion. */
export function orbitAppearance(identity: string, color = ""): { eyes: EyeStyle; smile: SmileStyle; freckles: boolean } {
  let hash = 2166136261;
  for (const character of `${identity}:${color}`) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  const eyes: EyeStyle[] = ["bright", "happy", "curious", "relaxed", "sparkle", "round"];
  const smiles: SmileStyle[] = ["smile", "grin", "cheeky", "beam", "soft"];
  return { eyes: eyes[hash % eyes.length], smile: smiles[Math.floor(hash / eyes.length) % smiles.length], freckles: (hash & 64) !== 0 };
}

/** A fresh face without renaming the Sprite or changing its workspace identity. */
export function generateOrbitAvatarSeed(identity: string, color: string, newSeed = () => crypto.randomUUID()): string {
  const current = orbitAppearance(identity, color);
  const seed = newSeed();
  for (let attempt = 0; attempt < 128; attempt += 1) {
    const candidate = attempt === 0 ? seed : `${seed}-${attempt}`;
    const appearance = orbitAppearance(candidate, color);
    if (candidate !== identity && (appearance.eyes !== current.eyes || appearance.smile !== current.smile || appearance.freckles !== current.freckles)) return candidate;
  }
  throw new Error("Could not create a new avatar. Please try again.");
}
