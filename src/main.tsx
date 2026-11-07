import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { DesktopWidget } from "./DesktopWidget";
import { AttentionWidget } from "./AttentionWidget";
import { installClickSounds } from "./soundEffects";
import "./styles.css";

function Root() {
  useEffect(installClickSounds, []);
  const [route, setRoute] = useState(window.location.hash.slice(1));
  useEffect(() => {
    const update = () => setRoute(window.location.hash.slice(1));
    window.addEventListener("hashchange", update);
    return () => window.removeEventListener("hashchange", update);
  }, []);
  const [type, id] = route.split("/");
  const widget = ["sun", "orbit", "chat", "attention"].includes(type);
  useEffect(() => {
    document.documentElement.classList.toggle("widget-root", widget);
  }, [widget]);
  return type === "attention" ? (
    <AttentionWidget />
  ) : widget ? (
    <DesktopWidget type={type as "sun" | "orbit" | "chat"} orbitId={id} />
  ) : (
    <App />
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
