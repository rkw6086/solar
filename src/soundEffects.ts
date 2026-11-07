import { api, native } from "./api";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type SoundCue = "click" | "ripple" | "formation" | "wink";

function variation() {
  return crypto.getRandomValues(new Uint32Array(1))[0];
}

export function playSoundEffect(cue: SoundCue, variant = variation()) {
  if (!native || document.visibilityState !== "visible") return;
  void api("play_sound_effect", { cue, variant }).catch(() => {});
}

export function stopSoundEffects() {
  // The launcher owns intro playback. Hiding a companion must not stop it.
  if (native && getCurrentWindow().label === "main") {
    void api("stop_sound_effects").catch(() => {});
  }
}

/** One sound for an activated control, including keyboard-triggered clicks. */
export function installClickSounds() {
  const clicked = (event: MouseEvent) => {
    if (!(event.target instanceof Element)) return;
    const control = event.target.closest("button, a[href], input, select, summary, [role='button'], [role='switch']");
    if (!control || control.matches(":disabled, [aria-disabled='true']")) return;
    if (control instanceof HTMLInputElement && !["checkbox", "radio", "range", "button", "submit"].includes(control.type)) return;
    playSoundEffect("click");
  };
  const hidden = () => {
    if (document.visibilityState !== "visible") stopSoundEffects();
  };
  document.addEventListener("click", clicked, true);
  document.addEventListener("visibilitychange", hidden);
  return () => {
    document.removeEventListener("click", clicked, true);
    document.removeEventListener("visibilitychange", hidden);
  };
}
