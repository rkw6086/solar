import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { DesktopWidget } from "./DesktopWidget";
import "./styles.css";

function Root() {
  const [route, setRoute] = useState(window.location.hash.slice(1));
  useEffect(() => {
    const update = () => setRoute(window.location.hash.slice(1));
    window.addEventListener("hashchange", update);
    return () => window.removeEventListener("hashchange", update);
  }, []);
  const [type, id] = route.split("/");
  const widget = type === "sun" || type === "orbit" || type === "chat";
  useEffect(() => {
    document.documentElement.classList.toggle("widget-root", widget);
  }, [widget]);
  return widget ? (
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
