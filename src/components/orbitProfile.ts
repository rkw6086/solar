import type { Orbit } from "../types";

/** Keep this comparison aligned with the native profile command. */
export function orbitProfileNeedsGeneration(orbit: Orbit, original?: Orbit): boolean {
  if (!original) return true;
  return (["name", "role", "personality", "tasks"] as const)
    .some((key) => (orbit[key] ?? "").trim() !== (original[key] ?? "").trim());
}
