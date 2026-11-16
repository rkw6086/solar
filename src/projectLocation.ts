export interface ProjectLocation {
  directory: string;
  name: string;
  isGitRoot: boolean;
}

/** Preview-only path naming; native Solar resolves actual folders and Git roots. */
export function previewProjectLocation(value: string): ProjectLocation {
  const directory = value.trim().replace(/[\\/]+$/, "");
  const name = directory.split(/[\\/]/).at(-1) ?? "";
  if (!directory || !name || name === "." || name === ".." || /^[A-Za-z]:$/.test(name)) {
    throw new Error("Choose a project folder with a name.");
  }
  return { directory, name, isGitRoot: false };
}
