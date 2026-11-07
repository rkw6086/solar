import { useMemo } from "react";

interface PlanetProps {
  color?: string;
  name?: string;
  size?: number;
  sun?: boolean;
  busy?: boolean;
  className?: string;
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
}: PlanetProps) {
  const pixels = useMemo(() => {
    const cells: { x: number; y: number; color: string }[] = [];
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
          cells.push({ x, y, color: shade(color, amount) });
        } else if (
          sun &&
          distance < 11.6 &&
          distance > 9 &&
          ((x + y) % 5 === 0 || x === 11 || y === 11)
        ) {
          cells.push({ x, y, color: shade(color, -20) });
        }
      }
    }
    return cells;
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
      {pixels.map((cell) => (
        <rect
          key={`${cell.x}-${cell.y}`}
          x={cell.x}
          y={cell.y}
          width="1"
          height="1"
          fill={cell.color}
        />
      ))}
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
      <g className="planet-eyes" fill="#20252a">
        <rect x="8" y="11" width="1" height="2" />
        <rect x="14" y="11" width="1" height="2" />
      </g>
      <path
        d="M10 14h1v1h2v-1h1"
        fill="none"
        stroke="#20252a"
        strokeWidth=".7"
      />
      <rect x="6" y="13" width="2" height="1" fill="#df8c81" opacity=".45" />
      <rect x="15" y="13" width="2" height="1" fill="#df8c81" opacity=".45" />
    </svg>
  );
}
