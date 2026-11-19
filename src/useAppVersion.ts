import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import packageInfo from "../package.json";
import { native } from "./api";

export function useAppVersion() {
  const [version, setVersion] = useState<string | null>(
    native ? null : packageInfo.version,
  );

  useEffect(() => {
    if (!native) return;
    let active = true;
    void getVersion().then(
      (value) => { if (active) setVersion(value); },
      () => { if (active) setVersion(null); },
    );
    return () => { active = false; };
  }, []);

  if (!version || import.meta.env.VITE_SOLAR_BUILD_CHANNEL !== "beta") return version;
  const commit = import.meta.env.VITE_SOLAR_SOURCE_COMMIT;
  const identity = typeof commit === "string" && /^[0-9a-f]{40}$/.test(commit)
    ? commit.slice(0, 12) : "unknown commit";
  return `${version} · Beta · ${identity}`;
}
