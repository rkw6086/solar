/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_SOLAR_BUILD_CHANNEL?: "release" | "beta";
  readonly VITE_SOLAR_SOURCE_COMMIT?: string;
}
