import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { Plus, Sparkles } from "lucide-react";
import type { Orbit } from "../types";
import { Planet } from "./Planet";
import { orbitPosition, orbitTracks } from "./solarSystemMotion";
import "./solarSystem.css";

interface SolarSystemProps {
  orbits: Orbit[];
  onSelectOrbit: (orbit: Orbit) => void;
  onCreateOrbit: () => void;
  reducedMotion?: boolean;
  compact?: boolean;
}

export function SolarSystem({
  orbits,
  onSelectOrbit,
  onCreateOrbit,
  reducedMotion = false,
  compact = false,
}: SolarSystemProps) {
  const stage = useRef<HTMLDivElement>(null);
  const planets = useRef(new Map<string, HTMLButtonElement>());
  const elapsed = useRef(0);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const tracks = useMemo(() => orbitTracks(orbits), [orbits]);

  useEffect(() => {
    const element = stage.current;
    if (!element || !tracks.length) return;
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    let frame = 0;
    let previous: number | undefined;
    let width = element.clientWidth;
    let height = element.clientHeight;
    let disposed = false;

    const paint = () => {
      for (const track of tracks) {
        const { x, y } = orbitPosition(track, elapsed.current, width, height);
        planets.current.get(track.orbit.id)?.style.setProperty(
          "transform",
          `translate(${x}px, ${y}px) translate(-50%, -50%)`,
        );
      }
    };
    const shouldAnimate = () =>
      !disposed && !document.hidden && !media.matches && !reducedMotion &&
      document.documentElement.dataset.motion !== "reduced" && !hovered && !focused;
    const tick = (now: number) => {
      if (!shouldAnimate()) {
        frame = 0;
        previous = undefined;
        return;
      }
      if (previous !== undefined) elapsed.current += now - previous;
      previous = now;
      paint();
      frame = window.requestAnimationFrame(tick);
    };
    const reconcile = () => {
      if (!shouldAnimate()) {
        window.cancelAnimationFrame(frame);
        frame = 0;
        previous = undefined;
      } else if (!frame) {
        frame = window.requestAnimationFrame(tick);
      }
      paint();
    };
    const resize = new ResizeObserver(() => {
      width = element.clientWidth;
      height = element.clientHeight;
      paint();
    });
    const preferences = new MutationObserver(reconcile);
    resize.observe(element);
    preferences.observe(document.documentElement, { attributes: true, attributeFilter: ["data-motion"] });
    media.addEventListener("change", reconcile);
    document.addEventListener("visibilitychange", reconcile);
    reconcile();
    return () => {
      disposed = true;
      window.cancelAnimationFrame(frame);
      resize.disconnect();
      preferences.disconnect();
      media.removeEventListener("change", reconcile);
      document.removeEventListener("visibilitychange", reconcile);
    };
  }, [tracks, hovered, focused, reducedMotion]);

  const empty = !tracks.length;
  return (
    <section className={`solar-system ${empty ? "solar-system-empty" : ""} ${compact ? "solar-system-compact" : ""}`} aria-labelledby="solar-system-title">
      <header className="solar-system-heading">
        <span className="solar-system-eyebrow"><Sparkles size={13} /> YOUR SOLAR SYSTEM</span>
        <h1 id="solar-system-title">{empty ? "It starts with one Orbit." : "A little universe of your own."}</h1>
        <p>{empty
          ? "Give your first Orbit a name, a specialty, and a personality. Build your team from there."
          : "Your Orbits, moving together around Solar. Choose one to start a conversation."}</p>
      </header>

      <div
        className={`solar-system-stage ${tracks.length > 8 ? "solar-system-crowded" : ""}`}
        ref={stage}
        onFocusCapture={() => setFocused(true)}
        onBlurCapture={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false);
        }}
        aria-label={empty ? "Solar is ready for your first Orbit" : "Your Orbits around Solar"}
      >
        <div className="solar-system-dust" aria-hidden="true" />
        <div className="solar-system-halo" aria-hidden="true" />
        {tracks.map(({ orbit, x, y }) => (
          <div
            key={orbit.id}
            className="solar-system-track"
            aria-hidden="true"
            style={{ width: `${x * 200}%`, height: `${y * 200}%`, "--track-color": orbit.color } as CSSProperties}
          />
        ))}
        <div className="solar-system-star">
          <Planet sun color="#e8ad66" name="Solar" size={112} />
          <span>Solar</span>
        </div>
        {tracks.map(({ orbit }) => (
          <button
            type="button"
            className="solar-system-orbit"
            key={orbit.id}
            ref={(element) => {
              if (element) planets.current.set(orbit.id, element);
              else planets.current.delete(orbit.id);
            }}
            onClick={() => onSelectOrbit(orbit)}
            onPointerEnter={() => setHovered(true)}
            onPointerLeave={() => setHovered(false)}
            aria-label={`Chat with ${orbit.name}, ${orbit.role}`}
            title={`${orbit.name} · ${orbit.role}`}
          >
            <Planet color={orbit.color} name={orbit.name} size={58} />
            <span className="solar-system-orbit-name">{orbit.name}</span>
          </button>
        ))}
      </div>

      <footer className="solar-system-footer">
        <button type="button" className="button button-primary" onClick={onCreateOrbit}>
          <Plus size={16} />{empty ? "Create your first Orbit" : "Create an Orbit"}
        </button>
        <span>{empty ? "You choose who joins your team." : "In resonance · a shared rhythm, at different speeds"}</span>
      </footer>
    </section>
  );
}
