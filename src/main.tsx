import { lazy, StrictMode, Suspense, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { installClickSounds } from "./soundEffects";
import { installDiagnostics, reportDiagnostic } from "./diagnostics";
import "./styles.css";

// Each native window loads only its own surface, with common modules shared
// between chunks instead of one large eagerly loaded application bundle.
const App = lazy(() => import("./App").then(module => ({ default: module.App })));
const DesktopWidget = lazy(() => import("./DesktopWidget").then(module => ({ default: module.DesktopWidget })));
const AttentionWidget = lazy(() => import("./AttentionWidget").then(module => ({ default: module.AttentionWidget })));

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
  const surface = type === "attention" ? (
    <AttentionWidget />
  ) : widget ? (
    <DesktopWidget type={type as "sun" | "orbit" | "chat"} orbitId={id} />
  ) : (
    <App />
  );
  return <Suspense fallback={widget ? null : <div className="startup-black" aria-busy="true" />}>{surface}</Suspense>;
}

installDiagnostics();

createRoot(document.getElementById("root")!, {
  onUncaughtError: (error) => reportDiagnostic("react_error", error),
  onCaughtError: (error) => reportDiagnostic("react_error", error),
  onRecoverableError: (error) => reportDiagnostic("react_error", error),
}).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
