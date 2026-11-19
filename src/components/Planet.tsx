import { useMemo } from "react";
import { orbitAppearance } from "./orbitIdentity";
import type { EyeStyle, SmileStyle } from "./orbitIdentity";

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

function shade(hex: string, amount: number) {
  const value = /^#[0-9a-f]{6}$/i.test(hex) ? hex.slice(1) : "79bcb5";
  const channels = [0, 2, 4].map((offset) =>
    Math.max(
      0,
      Math.min(255, parseInt(value.slice(offset, offset + 2), 16) + amount),
    ),
  );
  return `rgb(${channels.join(",")})`;
}

export function Planet({
  color = "#79bcb5",
  name = "Orbit",
  size = 88,
  sun = false,
  busy = false,
  className = "",
  identity,
  mood,
}: PlanetProps) {
  const appearance = orbitAppearance(identity ?? name, color);
  const eyeStyle: EyeStyle = sun ? "bright" : appearance.eyes;
  const smileStyle: SmileStyle = sun ? "smile" : appearance.smile;
  const pixels = useMemo(() => {
    const body: { x: number; y: number; color: string }[] = [];
    const flares: { x: number; y: number; color: string }[] = [];
    for (let y = 0; y < 24; y += 1) {
      for (let x = 0; x < 24; x += 1) {
        const dx = x - 11.5;
        const dy = y - 11.5;
        const radius = sun ? 7.8 : 8.7;
        const distance = Math.sqrt(dx * dx + dy * dy);
        if (distance <= radius) {
          let amount = -Math.round((dx + dy) * 2.6);
          if (x < 7 && y < 10) amount += 24;
          if (distance > radius - 1) amount -= 30;
          body.push({ x, y, color: shade(color, amount) });
        } else if (
          sun &&
          distance < 11.6 &&
          distance > 9 &&
          ((x + y) % 5 === 0 || x === 11 || y === 11)
        ) {
          flares.push({ x, y, color: shade(color, -20) });
        }
      }
    }
    return { body, flares };
  }, [color, sun]);

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      className={`planet-art ${sun ? "sun-art" : ""} ${busy ? "is-busy" : ""} ${className}`}
      role="img"
      aria-label={name}
      shapeRendering="crispEdges"
    >
      {sun && <g className="sun-flares" aria-hidden="true"><g className="sun-flare-pixels">
        {pixels.flares.map((cell) => (
          <rect key={`${cell.x}-${cell.y}`} x={cell.x} y={cell.y} width="1" height="1" fill={cell.color} />
        ))}
      </g></g>}
      <g className="planet-body">{pixels.body.map((cell) => (
        <rect
          key={`${cell.x}-${cell.y}`}
          x={cell.x}
          y={cell.y}
          width="1"
          height="1"
          fill={cell.color}
        />
      ))}</g>
      {!sun && (
        <>
          <rect
            x="7"
            y="7"
            width="3"
            height="1"
            fill={shade(color, 44)}
            opacity=".7"
          />
          <rect
            x="6"
            y="8"
            width="2"
            height="2"
            fill={shade(color, 44)}
            opacity=".7"
          />
        </>
      )}
      <g className="planet-face" data-eye-style={eyeStyle} data-smile-style={smileStyle}>
        <g className="planet-eyes" fill="#20252a" stroke="#20252a" strokeWidth=".65">
          {mood === "yawn" || eyeStyle === "happy" ? <path d="M7.5 12l1-1 1 1M13.5 12l1-1 1 1" fill="none" />
            : eyeStyle === "relaxed" ? <path d="M7.5 11.5h2M13.5 11.5h2" />
            : eyeStyle === "curious" ? <><rect x="8" y="11" width="1" height="2" stroke="none" /><rect x="14" y="10.5" width="1.5" height="2.5" stroke="none" /><path d="M13.5 9h2" fill="none" /></>
            : eyeStyle === "sparkle" ? <path d="M8.5 10.5v3M7 12h3M14.5 10.5v3M13 12h3" />
            : eyeStyle === "round" ? <><rect x="7.5" y="11" width="2" height="2" stroke="none" /><rect x="13.5" y="11" width="2" height="2" stroke="none" /><path d="M8 11v.5M14 11v.5" stroke="#fff" strokeWidth=".6" /></>
            : <><rect x="8" y="11" width="1" height="2" stroke="none" /><rect x="14" y="11" width="1" height="2" stroke="none" /></>}
        </g>
        <g className="planet-mouth" stroke="#20252a" strokeWidth=".7" fill="none">
          {mood === "yawn" ? <ellipse cx="11.5" cy="15" rx="1.2" ry="1.7" fill="#20252a" />
            : mood === "neutral" ? <path d="M10 14.5h3" />
            : smileStyle === "grin" ? <><path d="M9 14h5v1l-1 1h-3l-1-1z" fill="#20252a" /><path d="M10 14.5h3" stroke="#fff4da" strokeWidth=".6" /></>
            : smileStyle === "cheeky" ? <path d="M10 14.5h2l2-1M14 13v1.5" />
            : smileStyle === "beam" ? <path d="M9 14l1 2h3l1-2z" fill="#20252a" />
            : smileStyle === "soft" ? <path d="M10 14.5l1.5.5 1.5-.5" />
            : <path d="M10 14h1v1h2v-1h1" />}
        </g>
      </g>
      {!sun && appearance.freckles && <g className="planet-freckles" fill={shade(color, -70)} opacity=".65">
        <rect x="6" y="13" width=".5" height=".5" /><rect x="7" y="13.5" width=".5" height=".5" />
        <rect x="16" y="13" width=".5" height=".5" /><rect x="15" y="13.5" width=".5" height=".5" />
      </g>}
      <rect x="6" y="13" width="2" height="1" fill="#df8c81" opacity=".45" />
      <rect x="15" y="13" width="2" height="1" fill="#df8c81" opacity=".45" />
    </svg>
  );
}
