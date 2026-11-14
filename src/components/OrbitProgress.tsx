import { useEffect, useRef, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { Planet } from "./Planet";
import "./orbitProgress.css";

const messages = [
  "Giving your Orbit a little space to think.",
  "Polishing a few bright ideas. Sunglasses optional.",
  "Checking the rings. Saturn takes this personally.",
  "Putting the personality in planetary.",
  "Making room for one more star on the team.",
  "Briefly consulting the Department of Cosmic Curiosity.",
  "Orbit check: all systems curious. No astronaut training required.",
];

export function OrbitProgress({ name, color, creating }: {
  name: string; color: string; creating: boolean;
}) {
  const [message, setMessage] = useState(0);
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    panel.current?.focus();
    const timer = window.setInterval(() => setMessage((value) => (value + 1) % messages.length), 4200);
    return () => window.clearInterval(timer);
  }, []);
  return <div className="orbit-progress-backdrop" onMouseDown={(event) => event.stopPropagation()}>
    <div className="orbit-progress-dialog" role="dialog" aria-modal="true"
      aria-labelledby="orbit-progress-title" aria-describedby="orbit-progress-description"
      tabIndex={-1} ref={panel} onKeyDown={(event) => {
        if (event.key === "Escape" || event.key === "Tab") { event.preventDefault(); event.stopPropagation(); }
      }}>
      <div className="orbit-progress-planet"><Planet name={name || "Your Orbit"} color={color} size={76} /></div>
      <h2 id="orbit-progress-title">{creating ? `Getting ${name || "your Orbit"} ready` : `Updating ${name || "your Orbit"}`}</h2>
      <p id="orbit-progress-description">Shaping their instructions from your ideas.</p>
      <div className="orbit-progress-status" role="status" aria-live="polite">
        <LoaderCircle size={19} className="spin" aria-hidden="true" />
        <span key={message}>{messages[message]}</span>
      </div>
    </div>
  </div>;
}
