export const sunAntics = ["roll", "yawn", "rocket", "comet", "ufo"] as const;
export type SunAntic = (typeof sunAntics)[number];

// Leave a little quiet time between visits, and never repeat the same greeting.
export function nextSunAntic(previous: SunAntic | null, random = Math.random): SunAntic {
  const choices = sunAntics.filter(antic => antic !== previous);
  const index = Math.min(choices.length - 1, Math.max(0, Math.floor(random() * choices.length)));
  return choices[index];
}

export function sunAnticDelay(random = Math.random): number {
  return 18000 + Math.round(Math.min(1, Math.max(0, random())) * 14000);
}

export function sunAnticDuration(antic: SunAntic): number {
  return antic === "ufo" ? 5800 : antic === "rocket" ? 5200 : antic === "comet" ? 4800 : 3200;
}
