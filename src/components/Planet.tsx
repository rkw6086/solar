import { memo, useMemo } from "react";
import type { CSSProperties } from "react";
import { orbitAppearance } from "./orbitIdentity";
import type { EyeStyle, OrbitAppearance, SmileStyle } from "./orbitIdentity";
import "./planet.css";

interface PlanetProps {
  color?: string;
  name?: string;
  size?: number;
  sun?: boolean;
  busy?: boolean;
  className?: string;
  identity?: string;
  mood?: "smile" | "yawn" | "neutral";
}

function channels(hex: string) {
  const value = /^#[0-9a-f]{6}$/i.test(hex) ? hex.slice(1) : "79bcb5";
  return [0, 2, 4].map((offset) => parseInt(value.slice(offset, offset + 2), 16));
}

function shade(hex: string, amount: number, accent?: string, mix = 0) {
  const base = channels(hex);
  const other = accent ? channels(accent) : base;
  return `rgb(${base.map((value, index) => Math.round(Math.max(0, Math.min(255,
    value * (1 - mix) + other[index] * mix + amount,
  )))).join(",")})`;
}

/** Broad, quiet surface shapes stay legible behind a face, even at icon sizes. */
function surface(x: number, y: number, appearance: OrbitAppearance) {
  const phase = appearance.variation;
  const face = x >= 7 && x <= 16 && y >= 10 && y <= 16;
  const soften = face ? 0.35 : 1;
  let amount = 0;
  let accent: string | undefined;
  let mix = 0;
  switch (appearance.surface) {
    case "ocean": {
      const land = Math.sin((x + phase) * 0.65) + Math.cos(y * 0.7 - phase) + Math.sin((x + y) * 0.38);
      accent = land > 0.75 ? "#b5c988" : "#558fb3";
      mix = land > 0.75 ? 0.5 : 0.16;
      amount = land > 0.75 ? 7 : -5;
      break;
    }
    case "cloudy":
      amount = ((Math.floor(y / 3) + phase) % 3 === 0 ? 21 : -7);
      accent = "#f2e6da"; mix = amount > 0 ? 0.22 : 0;
      break;
    case "banded":
      amount = [16, -17, 4, -8][(Math.floor((y + Math.sin(x * 0.3 + phase)) / 2) + phase + 10) % 4];
      accent = "#b798cb"; mix = amount < -10 ? 0.18 : 0;
      break;
    case "cratered": {
      const craters = [[6, 7, 2], [17, 7 + phase % 3, 2.4], [6 + phase % 2, 17, 1.8], [16, 18, 1.4]];
      for (const [cx, cy, radius] of craters) {
        const distance = Math.hypot(x - cx, y - cy);
        if (distance < radius) amount = -27;
        else if (distance < radius + 0.8) amount = x < cx ? 16 : -6;
      }
      break;
    }
    case "icy":
      accent = "#d5f2ed"; mix = 0.25;
      amount = ((x + y * 2 + phase) % 11 === 0 || (x * 2 - y + phase + 24) % 15 === 0) ? 29 : -4;
      break;
    case "lava": {
      const vein = Math.abs(Math.sin(x * 0.53 + phase) + Math.cos(y * 0.68)) < 0.22;
      accent = vein ? "#f6ae66" : "#724b68"; mix = vein ? 0.7 : 0.23;
      amount = vein ? 14 : -15;
      break;
    }
    case "crystal":
      amount = x + y < 17 + phase ? 20 : x - y > phase - 2 ? -19 : 3;
      accent = "#dfd2f4"; mix = 0.18;
      break;
    case "marbled":
      amount = Math.sin(x * 0.55 + Math.sin(y * 0.5 + phase) * 2.1) > 0.25 ? 17 : -13;
      accent = "#e6c38e"; mix = amount > 0 ? 0.18 : 0;
      break;
    default:
      amount = ((Math.floor(x / 3) * 7 + Math.floor(y / 3) * 11 + phase) % 5 - 2) * 7;
  }
  return { amount: amount * soften, accent, mix: mix * soften };
}

function Eyes({ style, ink, highlight }: { style: EyeStyle; ink: string; highlight: string }) {
  const dots = <><rect x="8" y="11" width="1" height="2" stroke="none" /><rect x="14" y="11" width="1" height="2" stroke="none" /></>;
  let drawing;
  switch (style) {
    case "happy": drawing = <path d="M7.5 12l1-1 1 1M13.5 12l1-1 1 1" fill="none" />; break;
    case "relaxed": drawing = <path d="M7.5 11.5h2M13.5 11.5h2" />; break;
    case "curious": drawing = <><rect x="8" y="11" width="1" height="2" stroke="none" /><rect x="14" y="10.5" width="1.5" height="2.5" stroke="none" /><path d="M13.5 9h2" fill="none" /></>; break;
    case "sparkle": drawing = <path d="M8.5 10.5v3M7 12h3M14.5 10.5v3M13 12h3" />; break;
    case "round": drawing = <><rect x="7.5" y="11" width="2" height="2" stroke="none" /><rect x="13.5" y="11" width="2" height="2" stroke="none" /><path d="M8 11v.5M14 11v.5" stroke={highlight} strokeWidth=".6" /></>; break;
    case "wink": drawing = <><rect x="8" y="11" width="1.5" height="2" stroke="none" /><path d="M13.5 11l2 1-2 1" fill="none" /></>; break;
    case "wide": drawing = <><rect x="7.5" y="10.5" width="2" height="3" rx=".3" stroke="none" /><rect x="13.5" y="10.5" width="2" height="3" rx=".3" stroke="none" /><path d="M8 11v1M14 11v1" stroke={highlight} /></>; break;
    case "sleepy": drawing = <><path d="M7.5 11h2v1h-2M13.5 11h2v1h-2" fill="none" /><path d="M7 9.5h2M14 9.5h2" opacity=".65" fill="none" /></>; break;
    case "focused": drawing = <><path d="M7 10l3 1M13 11l3-1" fill="none" />{dots}</>; break;
    case "lashes": drawing = <path d="M7.5 11.5l1 1 1-1M13.5 11.5l1 1 1-1M7 10.5l.5 1M16 10.5l-.5 1" fill="none" />; break;
    case "glasses": drawing = <><path d="M6.5 10.5h4v3h-4zM12.5 10.5h4v3h-4zM10.5 11.5h2" fill="none" strokeWidth=".5" />{dots}</>; break;
    case "star": drawing = <path d="M8.5 10l.5 1 1 .5-1 1-.5 1-.5-1-1-1 1-.5zM14.5 10l.5 1 1 .5-1 1-.5 1-.5-1-1-1 1-.5z" strokeWidth=".3" />; break;
    case "crescent": drawing = <path d="M8.5 10.5l-1 1.5 1 1M14.5 10.5l1 1.5-1 1" fill="none" />; break;
    default: drawing = dots;
  }
  return <g className="planet-eyes" fill={ink} stroke={ink} strokeWidth=".65" strokeLinejoin="round">{drawing}</g>;
}

function Mouth({ style, ink, highlight, mood }: { style: SmileStyle; ink: string; highlight: string; mood?: PlanetProps["mood"] }) {
  let drawing;
  if (mood === "yawn") drawing = <ellipse cx="11.5" cy="15" rx="1.2" ry="1.7" fill={ink} />;
  else if (mood === "neutral") drawing = <path d="M10 14.5h3" />;
  else switch (style) {
    case "grin": drawing = <><path d="M9 14h5v1l-1 1h-3l-1-1z" fill={ink} /><path d="M10 14.5h3" stroke={highlight} strokeWidth=".6" /></>; break;
    case "cheeky": drawing = <path d="M10 14.5h2l2-1M14 13v1.5" />; break;
    case "beam": drawing = <path d="M9 14l1 2h3l1-2z" fill={ink} />; break;
    case "soft": drawing = <path d="M10 14.5l1.5.5 1.5-.5" />; break;
    case "smirk": drawing = <path d="M9.5 15h2l2-1.5M13.5 13v1.5" />; break;
    case "open": drawing = <><path d="M10 14h3v2h-3z" fill={ink} /><path d="M10.5 15.7h2" stroke="#e9a9a0" strokeWidth=".7" /></>; break;
    case "delighted": drawing = <><path d="M9 14h5l-1 2.5h-3z" fill={ink} /><path d="M10 14.5h3" stroke={highlight} /></>; break;
    case "cat": drawing = <path d="M9 14.5l1 1 1.5-1 1.5 1 1-1M11.5 14v.5" />; break;
    case "tooth": drawing = <><path d="M9.5 14h4v2h-4z" fill={ink} /><path d="M11 14.3h1v.7h-1z" fill={highlight} stroke="none" /></>; break;
    case "surprised": drawing = <path d="M11 14h1v2h-1z" fill={ink} />; break;
    default: drawing = <path d="M10 14h1v1h2v-1h1" />;
  }
  return <g className="planet-mouth" stroke={ink} strokeWidth=".7" strokeLinejoin="round" fill="none">{drawing}</g>;
}

function Rings({ appearance, color, front = false }: { appearance: OrbitAppearance; color: string; front?: boolean }) {
  if (appearance.rings === "none") return null;
  const rotation = appearance.rings === "flat" ? 0 : appearance.variation % 2 ? 17 : -17;
  return <g className={`planet-rings planet-rings-${front ? "front" : "back"}`} transform={`rotate(${rotation} 12 12)`} fill="none" stroke={shade(color, 36, "#f4d8b0", 0.35)} strokeWidth=".9" opacity={front ? ".95" : ".7"}>
    {front ? <path d="M1 12C1 19 23 19 23 12" /> : <ellipse cx="12" cy="12" rx="11" ry="5.25" />}
    {appearance.rings === "double" && (front ? <path d="M.5 12C.5 20.5 23.5 20.5 23.5 12" strokeWidth=".45" /> : <ellipse cx="12" cy="12" rx="11.5" ry="6.35" strokeWidth=".45" />)}
  </g>;
}

export const Planet = memo(function Planet({ color = "#79bcb5", name = "Sprite", size = 88, sun = false, busy = false, className = "", identity, mood }: PlanetProps) {
  const appearance = useMemo(() => orbitAppearance(identity ?? name, color), [identity, name, color]);
  const eyeStyle: EyeStyle = sun ? "bright" : appearance.eyes;
  const smileStyle: SmileStyle = sun ? "smile" : appearance.smile;
  const ink = channels(color).reduce((sum, value, index) => sum + value * [0.21, 0.72, 0.07][index], 0) < 95 ? "#fff0d3" : "#20252a";
  const highlight = ink === "#20252a" ? "#fff4da" : "#253037";
  const pixels = useMemo(() => {
    const body: { x: number; y: number; color: string }[] = [];
    const flares: { x: number; y: number; color: string }[] = [];
    for (let y = 0; y < 24; y += 1) for (let x = 0; x < 24; x += 1) {
      const dx = x - 11.5, dy = y - 11.5;
      const radius = sun ? 7.8 : 8.7;
      const oblate = !sun && ["banded", "cloudy"].includes(appearance.surface);
      const distance = Math.hypot(dx / (oblate ? 1.04 : 1), dy / (oblate ? 0.94 : 1));
      const edge = !sun && appearance.surface === "crystal" ? Math.max(Math.abs(dx), Math.abs(dy)) * 0.5 + distance * 0.5 : distance;
      if (edge <= radius) {
        let amount = -Math.round((dx + dy) * 2.6);
        if (x < 7 && y < 10) amount += 24;
        if (edge > radius - 1) amount -= 30;
        const pattern = sun ? { amount: 0, accent: undefined, mix: 0 } : surface(x, y, appearance);
        body.push({ x, y, color: shade(color, amount + pattern.amount, pattern.accent, pattern.mix) });
      } else if (sun && distance < 11.6 && distance > 9 && ((x + y) % 5 === 0 || x === 11 || y === 11)) {
        flares.push({ x, y, color: shade(color, -20) });
      }
    }
    return { body, flares };
  }, [appearance, color, sun]);

  return <svg width={size} height={size} viewBox="0 0 24 24"
    className={`planet-art ${sun ? "sun-art" : ""} ${busy ? "is-busy" : ""} ${className}`}
    style={{ "--planet-blink-delay": `${-appearance.variation * 0.81}s` } as CSSProperties}
    role="img" aria-label={name} shapeRendering="crispEdges"
    data-planet-style={sun ? "sun" : appearance.surface} data-ring-style={sun ? "none" : appearance.rings}>
    {sun && <g className="sun-flares" aria-hidden="true"><g className="sun-flare-pixels">{pixels.flares.map(cell => <rect key={`${cell.x}-${cell.y}`} x={cell.x} y={cell.y} width="1" height="1" fill={cell.color} />)}</g></g>}
    {!sun && <Rings appearance={appearance} color={color} />}
    <g className="planet-body">{pixels.body.map(cell => <rect key={`${cell.x}-${cell.y}`} x={cell.x} y={cell.y} width="1" height="1" fill={cell.color} />)}</g>
    {!sun && <g className="planet-shine" fill={shade(color, 48)} opacity=".55"><rect x="7" y="7" width="3" height="1" /><rect x="6" y="8" width="2" height="2" /></g>}
    {!sun && <Rings appearance={appearance} color={color} front />}
    <g className="planet-face" data-eye-style={eyeStyle} data-smile-style={smileStyle}>
      <Eyes style={mood === "yawn" ? "happy" : eyeStyle} ink={ink} highlight={highlight} />
      <Mouth style={smileStyle} mood={mood} ink={ink} highlight={highlight} />
    </g>
    {!sun && appearance.freckles && <g className="planet-freckles" fill={ink} opacity=".45"><rect x="6" y="13" width=".5" height=".5" /><rect x="7" y="13.5" width=".5" height=".5" /><rect x="16" y="13" width=".5" height=".5" /><rect x="15" y="13.5" width=".5" height=".5" /></g>}
    {(sun || appearance.blush) && <g fill="#df8c81" opacity=".45"><rect x="6" y="13" width="2" height="1" /><rect x="15" y="13" width="2" height="1" /></g>}
  </svg>;
});
