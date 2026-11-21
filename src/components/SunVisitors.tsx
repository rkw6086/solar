import { useEffect, useId, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { native } from "../api";
import { nextSunAntic, sunAnticDelay, sunAnticDuration } from "./sunAntics";
import type { SunAntic } from "./sunAntics";
import "./sunVisitors.css";

export function useSunAntics(active: boolean, reducedMotion: boolean) {
  const [antic, setAntic] = useState<SunAntic | null>(null);

  useEffect(() => {
    setAntic(null);
    if (!active || reducedMotion) return;
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    let disposed = false;
    let epoch = 0;
    let previous: SunAntic | null = null;
    let start: ReturnType<typeof setTimeout> | undefined;
    let finish: ReturnType<typeof setTimeout> | undefined;
    let visibility: ReturnType<typeof setInterval> | undefined;

    const allowed = () => !disposed && !document.hidden && !media.matches;
    const visible = async () => allowed() && (!native || await getCurrentWindow().isVisible().catch(() => false));
    const clearPlaying = () => {
      clearTimeout(finish);
      clearInterval(visibility);
      if (!disposed) setAntic(null);
    };
    const queue = () => {
      clearTimeout(start);
      if (allowed()) start = setTimeout(() => { void play(); }, sunAnticDelay());
    };
    const play = async () => {
      const revision = ++epoch;
      if (!await visible()) {
        if (!disposed && epoch === revision) queue();
        return;
      }
      if (disposed || epoch !== revision) return;
      const choice = nextSunAntic(previous);
      previous = choice;
      setAntic(choice);
      finish = setTimeout(() => { clearPlaying(); queue(); }, sunAnticDuration(choice));
      // Native hidden windows do not consistently update document.hidden.
      // Poll only during a short visit, and ignore replies from an old visit.
      visibility = setInterval(() => {
        void visible().then(shown => {
          if (!shown && !disposed && epoch === revision) {
            epoch += 1;
            clearPlaying();
            queue();
          }
        });
      }, 750);
    };
    const reconcile = () => {
      epoch += 1;
      clearTimeout(start);
      clearPlaying();
      queue();
    };
    document.addEventListener("visibilitychange", reconcile);
    media.addEventListener("change", reconcile);
    queue();
    return () => {
      disposed = true;
      epoch += 1;
      clearTimeout(start);
      clearPlaying();
      document.removeEventListener("visibilitychange", reconcile);
      media.removeEventListener("change", reconcile);
    };
  }, [active, reducedMotion]);

  return {
    className: antic === "roll" ? "sun-roll" : antic === "yawn" ? "sun-yawn" : "",
    mood: antic === "yawn" ? "yawn" as const : antic ? "smile" as const : "neutral" as const,
    visitor: antic === "rocket" || antic === "comet" || antic === "ufo" ? antic : null,
  };
}

export function SunVisitors({ kind }: { kind: "rocket" | "comet" | "ufo" | null }) {
  const gradient = `comet-tail-${useId().replaceAll(":", "")}`;
  if (!kind) return null;
  return <div className={`sun-visitors sun-visitors-${kind}`} aria-hidden="true">
    {kind === "rocket" && <div className="sun-visitor-orbit">
      <svg className="sun-visitor-rocket" viewBox="0 0 20 28" shapeRendering="crispEdges" focusable="false">
        <path className="sun-rocket-flame" fill="#efb66c" d="M7 19h6v4h-1v3h-2v2H9v-3H8v-2H7z" />
        <path fill="#ffe7ad" d="M9 19h2v5H9z" />
        <path fill="#ebeee6" d="M8 1h4v2h2v3h1v12H5V6h1V3h2z" />
        <path fill="#bdccd2" d="M12 6h3v12h-3z" />
        <path fill="#d68d83" d="M8 1h4v2h2v3H6V3h2zM3 13h3v8H1v-5h2zm11 0h3v3h2v5h-5z" />
        <path fill="#416776" d="M7 8h6v6H7z" /><path fill="#93dcdd" d="M8 9h4v4H8z" />
        <path fill="#e8faf0" d="M8 9h2v2H8z" /><path fill="#8e9ca3" d="M5 18h10v2H5z" />
      </svg>
    </div>}
    {kind === "comet" && <svg className="sun-visitor-comet" viewBox="0 0 100 32" focusable="false">
      <defs><linearGradient id={gradient}><stop stopColor="#9acddd" stopOpacity="0" /><stop offset=".7" stopColor="#b4dfe1" stopOpacity=".7" /><stop offset="1" stopColor="#fff0c3" /></linearGradient></defs>
      <path d="M1 4 83 9 91 16 83 23 1 29 42 18 5 15 49 13z" fill={`url(#${gradient})`} />
      <path d="m35 7 48 7M25 24l55-6" stroke="#e4eece" strokeOpacity=".6" strokeWidth="2" strokeLinecap="round" />
      <circle cx="84" cy="16" r="11" fill="#acd9df" fillOpacity=".15" /><circle cx="84" cy="16" r="8" fill="#fff0c3" />
      <path d="M83 13v2m5-2v2" stroke="#737276" strokeWidth="1.5" strokeLinecap="round" /><path d="M84 18q2 2 4 0" fill="none" stroke="#bd947e" strokeWidth="1.2" strokeLinecap="round" />
    </svg>}
    {kind === "ufo" && <svg className="sun-visitor-ufo" viewBox="0 0 60 70" focusable="false">
      <path className="sun-ufo-beam" d="M23 29h14l15 35H8z" fill="#ccebb3" fillOpacity=".16" />
      <path d="M14 23V16C14 3 46 3 46 16v7" fill="#b4dce0" fillOpacity=".8" stroke="#8bbfc9" strokeWidth="1.5" />
      <path d="M23 22v-7c0-8 14-8 14 0v7" fill="#b8d498" />
      <path d="m25 13 3 2m7-2-3 2" stroke="#46545b" strokeWidth="2" strokeLinecap="round" /><path d="M28 19q2 2 4 0" fill="none" stroke="#647865" strokeWidth="1.2" strokeLinecap="round" />
      <ellipse cx="30" cy="25" rx="27" ry="8" fill="#a1acbd" /><path d="M3 25q27 15 54 0" fill="#727b91" />
      <ellipse cx="30" cy="22" rx="22" ry="4" fill="#c5d0d3" />
      <g className="sun-ufo-lights" fill="#f0d9a2"><circle cx="12" cy="27" r="2" /><circle cx="24" cy="30" r="2" /><circle cx="36" cy="30" r="2" /><circle cx="48" cy="27" r="2" /></g>
    </svg>}
  </div>;
}
