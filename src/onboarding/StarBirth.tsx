import { useEffect, useRef, useState } from "react";
import "./star-birth.css";

export type StarBirthPhase = "gather" | "burst" | "solar";

interface StarBirthProps {
  reducedMotion: boolean;
  onComplete: () => void;
  onPhaseChange?: (phase: StarBirthPhase) => void;
}

const OPENING_DURATION = 10_400;
const BURST_AT = 6_800;
const SOLAR_AT = 8_100;

const clamp = (value: number) => Math.min(1, Math.max(0, value));
const smooth = (value: number) => {
  const t = clamp(value);
  return t * t * (3 - 2 * t);
};
const seeded = (index: number) => {
  const value = Math.sin(index * 127.1 + 311.7) * 43_758.5453;
  return value - Math.floor(value);
};

function circle(context: CanvasRenderingContext2D, x: number, y: number, radius: number) {
  context.beginPath();
  context.arc(x, y, Math.max(0.01, radius), 0, Math.PI * 2);
}

function glow(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
  opacity: number,
  warm = false,
) {
  if (opacity <= 0 || radius <= 0) return;
  const gradient = context.createRadialGradient(x, y, 0, x, y, radius);
  gradient.addColorStop(0, `rgba(255,${warm ? 196 : 255},${warm ? 109 : 255},${opacity})`);
  gradient.addColorStop(0.22, `rgba(255,${warm ? 165 : 255},${warm ? 70 : 255},${opacity * 0.28})`);
  gradient.addColorStop(1, "rgba(0,0,0,0)");
  context.fillStyle = gradient;
  context.fillRect(x - radius, y - radius, radius * 2, radius * 2);
}

function drawStar(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
  time: number,
  opacity: number,
  face: number,
) {
  context.save();
  context.globalAlpha = opacity;
  glow(context, x, y, radius * 5.6, 0.3, true);

  // A quiet, irregular corona makes this feel like a star before the familiar
  // Solar expression emerges. It never illuminates the whole viewport.
  for (let ray = 0; ray < 22; ray += 1) {
    const angle = (ray / 22) * Math.PI * 2 + time * 0.025;
    const length = radius * (1.28 + seeded(ray + 70) * 0.55);
    const start = radius * 0.94;
    const gradient = context.createLinearGradient(
      x + Math.cos(angle) * start,
      y + Math.sin(angle) * start,
      x + Math.cos(angle) * length,
      y + Math.sin(angle) * length,
    );
    gradient.addColorStop(0, "rgba(255,195,107,.62)");
    gradient.addColorStop(1, "rgba(255,151,49,0)");
    context.strokeStyle = gradient;
    context.lineWidth = radius * (0.06 + seeded(ray + 90) * 0.05);
    context.lineCap = "round";
    context.beginPath();
    context.moveTo(x + Math.cos(angle) * start, y + Math.sin(angle) * start);
    context.lineTo(x + Math.cos(angle) * length, y + Math.sin(angle) * length);
    context.stroke();
  }

  const surface = context.createRadialGradient(
    x - radius * 0.28,
    y - radius * 0.35,
    radius * 0.06,
    x,
    y,
    radius,
  );
  surface.addColorStop(0, "#fff0c9");
  surface.addColorStop(0.43, "#ffd790");
  surface.addColorStop(0.84, "#eeb068");
  surface.addColorStop(1, "#dc8c43");
  context.fillStyle = surface;
  circle(context, x, y, radius);
  context.fill();

  if (face > 0) {
    context.globalAlpha = opacity * face;
    context.fillStyle = "#5c3c29";
    const pixel = Math.max(1.5, radius * 0.065);
    context.fillRect(x - radius * 0.29, y - radius * 0.06, pixel, pixel * 1.8);
    context.fillRect(x + radius * 0.22, y - radius * 0.06, pixel, pixel * 1.8);
    context.strokeStyle = "#5c3c29";
    context.lineWidth = Math.max(1.5, radius * 0.035);
    context.lineCap = "square";
    context.beginPath();
    context.moveTo(x - radius * 0.09, y + radius * 0.19);
    context.lineTo(x - radius * 0.09, y + radius * 0.24);
    context.lineTo(x + radius * 0.1, y + radius * 0.24);
    context.lineTo(x + radius * 0.1, y + radius * 0.19);
    context.stroke();
  }
  context.restore();
}

function paint(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  elapsed: number,
  reducedMotion: boolean,
) {
  context.clearRect(0, 0, width, height);
  context.fillStyle = "#000";
  context.fillRect(0, 0, width, height);

  const time = elapsed / 1000;
  const smaller = Math.min(width, height);
  const zoom = reducedMotion ? 1 : smooth((elapsed - 7_700) / 2_000);
  const centerX = width * 0.5;
  const centerY = height * (0.49 - zoom * 0.1);
  const afterglow = reducedMotion ? 1 : smooth((elapsed - BURST_AT) / 1_600);

  if (afterglow > 0) {
    for (let index = 0; index < 74; index += 1) {
      const x = seeded(index * 3 + 10) * width;
      const y = seeded(index * 3 + 11) * height;
      const away = Math.hypot(x - centerX, y - centerY) / smaller;
      const alpha = afterglow * (0.1 + seeded(index * 3 + 12) * 0.38) * clamp((away - 0.15) * 2);
      context.fillStyle = `rgba(230,229,224,${alpha})`;
      const size = seeded(index + 213) > 0.9 ? 1.5 : 0.8;
      context.fillRect(x, y, size, size);
    }
  }

  if (reducedMotion) {
    drawStar(context, centerX, centerY, smaller * 0.087, 0, 1, 1);
    return;
  }
  if (elapsed < 650) return;

  if (elapsed < BURST_AT) {
    const gathering = clamp((elapsed - 650) / 5_850);
    const compression = smooth((elapsed - 6_350) / 450);
    const maxRadius = Math.hypot(width, height) * 0.62;

    // Ripples arrive from beyond the window, and each leaves a little more
    // light at the center. Individual rings fade before they reach a point.
    for (let ring = 0; ring < 18; ring += 1) {
      const startsAt = 650 + ring * (330 - ring * 5);
      const progress = (elapsed - startsAt) / (2_650 - ring * 28);
      if (progress <= 0 || progress >= 1) continue;
      const radius = maxRadius * Math.pow(1 - progress, 1.65);
      const alpha = Math.sin(progress * Math.PI) * (0.16 + gathering * 0.27) * (1 - compression);
      context.strokeStyle = `rgba(243,245,248,${alpha})`;
      context.lineWidth = 0.65 + gathering * 0.6;
      circle(context, centerX, centerY, radius);
      context.stroke();
      context.strokeStyle = `rgba(221,231,245,${alpha * 0.16})`;
      context.lineWidth = 4;
      context.stroke();
    }

    for (let particle = 0; particle < 36; particle += 1) {
      const progress = ((time * 0.19 + seeded(particle + 600)) % 1);
      const angle = particle * 2.39996;
      const radius = smaller * 0.7 * Math.pow(1 - progress, 1.55);
      const alpha = Math.sin(progress * Math.PI) * gathering * 0.3 * (1 - compression);
      context.fillStyle = `rgba(245,247,250,${alpha})`;
      context.fillRect(centerX + Math.cos(angle) * radius, centerY + Math.sin(angle) * radius, 1.1, 1.1);
    }

    const nucleus = (2.5 + gathering * 8) * (1 - compression * 0.72);
    glow(context, centerX, centerY, 24 + gathering * 88, 0.12 + gathering * 0.22);
    context.fillStyle = `rgba(250,250,248,${0.2 + gathering * 0.75})`;
    circle(context, centerX, centerY, nucleus);
    context.fill();
  }

  if (elapsed >= BURST_AT) {
    const explosion = (elapsed - BURST_AT) / 1_650;
    if (explosion < 1.15) {
      const expansion = 1 - Math.pow(1 - clamp(explosion), 3);
      const fade = Math.pow(1 - clamp(explosion / 1.15), 2);
      glow(context, centerX, centerY, smaller * (0.12 + expansion * 0.38), fade * 0.5, true);
      for (let ring = 0; ring < 3; ring += 1) {
        const radius = smaller * (0.025 + expansion * (0.24 + ring * 0.105));
        context.strokeStyle = `rgba(255,${218 + ring * 10},${165 + ring * 25},${fade * (0.42 - ring * 0.09)})`;
        context.lineWidth = 1.5 - ring * 0.3;
        circle(context, centerX, centerY, radius);
        context.stroke();
      }
      for (let particle = 0; particle < 88; particle += 1) {
        const angle = particle * 2.39996;
        const distance = smaller * (0.13 + seeded(particle + 800) * 0.42) * expansion;
        const length = 2 + (1 - clamp(explosion)) * 13 * seeded(particle + 900);
        context.strokeStyle = `rgba(255,${205 + Math.round(seeded(particle + 720) * 40)},${137 + Math.round(seeded(particle + 730) * 90)},${fade * 0.75})`;
        context.lineWidth = 0.7 + seeded(particle + 950) * 0.8;
        context.beginPath();
        context.moveTo(centerX + Math.cos(angle) * distance, centerY + Math.sin(angle) * distance);
        context.lineTo(centerX + Math.cos(angle) * (distance + length), centerY + Math.sin(angle) * (distance + length));
        context.stroke();
      }
    }
    const starRadius = smaller * (0.033 + zoom * 0.054);
    drawStar(context, centerX, centerY, starRadius, time, smooth((elapsed - BURST_AT) / 320), smooth((elapsed - 8_500) / 700));
  }
}

export function StarBirth({ reducedMotion, onComplete, onPhaseChange }: StarBirthProps) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const container = useRef<HTMLElement>(null);
  const completed = useRef(false);
  const callbacks = useRef({ onComplete, onPhaseChange });
  callbacks.current = { onComplete, onPhaseChange };
  const currentPhase = useRef<StarBirthPhase | null>(null);
  const [phase, setPhase] = useState<StarBirthPhase>("gather");
  const [systemReduced, setSystemReduced] = useState(() =>
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  const calm = reducedMotion || systemReduced;

  function finish() {
    if (completed.current) return;
    completed.current = true;
    callbacks.current.onComplete();
  }

  useEffect(() => {
    const preference = window.matchMedia("(prefers-reduced-motion: reduce)");
    const changed = () => setSystemReduced(preference.matches);
    preference.addEventListener("change", changed);
    return () => preference.removeEventListener("change", changed);
  }, []);

  useEffect(() => {
    const element = canvas.current;
    const parent = container.current;
    if (!element || !parent) return;
    const context = element.getContext("2d");
    let width = 1;
    let height = 1;
    let elapsed = 0;
    let previous: number | null = null;
    let animation = 0;
    let disposed = false;

    const emitPhase = (value: StarBirthPhase) => {
      if (currentPhase.current === value) return;
      currentPhase.current = value;
      setPhase(value);
      callbacks.current.onPhaseChange?.(value);
    };
    emitPhase(calm ? "solar" : "gather");

    const resize = () => {
      const bounds = parent.getBoundingClientRect();
      width = Math.max(1, bounds.width);
      height = Math.max(1, bounds.height);
      const resolution = Math.min(window.devicePixelRatio || 1, 2);
      element.width = Math.round(width * resolution);
      element.height = Math.round(height * resolution);
      context?.setTransform(resolution, 0, 0, resolution, 0, 0);
      if (context) paint(context, width, height, elapsed, calm);
    };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(parent);

    const frame = (timestamp: number) => {
      if (disposed || completed.current || document.hidden) return;
      if (previous !== null) elapsed += Math.min(timestamp - previous, 80);
      previous = timestamp;
      const nextPhase = calm || elapsed >= SOLAR_AT ? "solar" : elapsed >= BURST_AT ? "burst" : "gather";
      emitPhase(nextPhase);
      if (context) paint(context, width, height, elapsed, calm);
      if (elapsed >= (calm ? 1_100 : OPENING_DURATION)) {
        if (!completed.current) {
          completed.current = true;
          callbacks.current.onComplete();
        }
        return;
      }
      animation = requestAnimationFrame(frame);
    };
    const visibility = () => {
      cancelAnimationFrame(animation);
      previous = null;
      if (!document.hidden && !disposed && !completed.current) animation = requestAnimationFrame(frame);
    };
    document.addEventListener("visibilitychange", visibility);
    if (!document.hidden) animation = requestAnimationFrame(frame);
    return () => {
      disposed = true;
      cancelAnimationFrame(animation);
      observer.disconnect();
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [calm]);

  return (
    <section ref={container} className={`star-birth${calm ? " star-birth-calm" : ""}`} data-phase={phase} aria-label="Welcome to Solar">
      <canvas ref={canvas} className="star-birth-canvas" aria-hidden="true" />
      <div className="star-birth-title" aria-live="polite">
        <span className="star-birth-kicker">A LITTLE LIGHT. A NEW BEGINNING.</span>
        <h1>Solar</h1>
        <p>Your universe of helpful minds.</p>
      </div>
      <p className="star-birth-description">White ripples gather into a new star. Meet Solar, your team of helpful Orbits.</p>
      <button className="star-birth-skip" onClick={finish} type="button">Skip opening <span aria-hidden="true">→</span></button>
    </section>
  );
}
