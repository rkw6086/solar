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

  return version;
}
