import type { Provider } from "../types";

export interface ModelOption {
  providerId: string;
  providerName: string;
  model: string;
  label: string;
}

export function modelOptions(
  providers: Pick<Provider, "id" | "name" | "models">[],
  current?: { providerId: string; model: string },
): ModelOption[] {
  const options: ModelOption[] = [];
  for (const provider of providers) {
    const models = new Set(provider.models.map((model) => model.trim()).filter(Boolean));
    if (current?.providerId === provider.id && current.model.trim()) {
      models.add(current.model.trim());
    }
    for (const model of models) {
      options.push({
        providerId: provider.id,
        providerName: provider.name,
        model,
        label: `${model} (${provider.name})`,
      });
    }
  }
  return options.sort((a, b) =>
    a.label.localeCompare(b.label, undefined, { sensitivity: "base", numeric: true }) ||
    a.providerId.localeCompare(b.providerId),
  );
}

function wildcardMatches(label: string, token: string): boolean {
  // A glob matcher avoids turning user queries into regular expressions.
  // Wrapping with stars gives ordinary tokens substring behavior as well.
  const pattern = [...`*${token.toLowerCase()}*`];
  const text = [...label.toLowerCase()];
  let index = 0;
  let cursor = 0;
  let star = -1;
  let retry = 0;
  while (index < text.length) {
    if (pattern[cursor] === "?" || pattern[cursor] === text[index]) {
      index += 1;
      cursor += 1;
    } else if (pattern[cursor] === "*") {
      star = cursor++;
      retry = index;
    } else if (star !== -1) {
      cursor = star + 1;
      index = ++retry;
    } else {
      return false;
    }
  }
  while (pattern[cursor] === "*") cursor += 1;
  return cursor === pattern.length;
}

export function filterModels(options: ModelOption[], query: string): ModelOption[] {
  const tokens = query.trim().split(/\s+/u).filter(Boolean);
  return tokens.length ? options.filter((option) => tokens.every((token) => wildcardMatches(option.label, token))) : options;
}
