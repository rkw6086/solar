import { useEffect, useState } from "react";
import { Copy, Minus, Square, Sun, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { native } from "../api";
import "./windowTitleBar.css";

/** Main-window chrome only. The intro and floating companions own their layout. */
export function WindowTitleBar() {
  const [maximized, setMaximized] = useState(false);
  const [focused, setFocused] = useState(true);
  const [error, setError] = useState("");
  const mac = /Mac|iPhone|iPad/.test(navigator.platform);

  useEffect(() => {
    if (!native) return;
    const window = getCurrentWindow();
    let disposed = false;
    const cleanup: (() => void)[] = [];
    const refreshMaximized = () => void window.isMaximized().then((value) => {
      if (!disposed) setMaximized(value);
    }).catch(() => {});
    refreshMaximized();
    void window.isFocused().then((value) => { if (!disposed) setFocused(value); }).catch(() => {});
    void window.onResized(refreshMaximized).then((unlisten) => {
      if (disposed) unlisten(); else cleanup.push(unlisten);
    }).catch(() => {});
    void window.onFocusChanged(({ payload }) => { if (!disposed) setFocused(payload); }).then((unlisten) => {
      if (disposed) unlisten(); else cleanup.push(unlisten);
    }).catch(() => {});
    return () => { disposed = true; cleanup.forEach((unlisten) => unlisten()); };
  }, []);

  async function act(action: "minimize" | "maximize" | "close" | "drag") {
    if (!native) return;
    setError("");
    const window = getCurrentWindow();
    try {
      if (action === "minimize") await window.minimize();
      else if (action === "close") await window.close(); // Retains Solar's close-to-tray handler.
      else if (action === "drag") await window.startDragging();
      else {
        await window.toggleMaximize();
        setMaximized(await window.isMaximized());
      }
    } catch {
      setError(action === "drag" ? "Couldn’t move the window" : "Couldn’t update the window");
    }
  }

  const controls = <div className="window-controls" onMouseDown={(event) => event.stopPropagation()}>
    <button type="button" className="window-control window-close" aria-label="Close Solar window" title={native ? "Close to menu bar" : "Available in the desktop app"} disabled={!native} onClick={() => void act("close")}><X size={13} /></button>
    <button type="button" className="window-control window-minimize" aria-label="Minimize Solar window" title={native ? "Minimize" : "Available in the desktop app"} disabled={!native} onClick={() => void act("minimize")}><Minus size={13} /></button>
    <button type="button" className="window-control window-maximize" aria-label={maximized ? "Restore Solar window" : "Maximize Solar window"} title={native ? (maximized ? "Restore" : "Maximize") : "Available in the desktop app"} disabled={!native} onClick={() => void act("maximize")}>{maximized ? <Copy size={12} /> : <Square size={11} />}</button>
  </div>;

  return <header className={`window-titlebar ${mac ? "window-titlebar-mac" : "window-titlebar-standard"} ${focused ? "" : "window-titlebar-inactive"}`} aria-label="Solar window title bar"
    onMouseDown={(event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      void act(event.detail === 2 ? "maximize" : "drag");
    }}>
    <div className="window-title"><Sun size={13} aria-hidden="true" /><span>Solar</span></div>
    {error && <span className="window-action-error" role="alert">{error}</span>}
    {controls}
  </header>;
}
