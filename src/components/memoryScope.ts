import type { Memory } from "../types";
export const memoryOrbitIds = (memory: Memory) => [...new Set([...(memory.orbitIds ?? []), ...(memory.orbitId ? [memory.orbitId] : [])])];
export const memoryProjectIds = (memory: Memory) => [...new Set([...(memory.projectIds ?? []), ...(memory.projectId ? [memory.projectId] : [])])];
export const memoryScope = (memory: Memory) => memoryOrbitIds(memory).length ? "orbit" : memoryProjectIds(memory).length ? "project" : "system";
