import "@fontsource-variable/roboto-flex/full.css";
import "./app.css";
import { createApp } from "vue";
import App from "./App.vue";

// macOS keeps its native window (tauri.macos.conf.json: overlay title bar with the traffic lights), so the
// app draws no frame or window buttons of its own there.
// iOS/Android: no window at all, so the title bar becomes a plain app bar below the status bar (an iPad
// asks for the desktop site and reports itself as a Mac with a touch screen).
const ua = navigator.userAgent;
const mobile = /iPhone|iPad|iPod|Android/.test(ua) || (ua.includes("Mac OS X") && navigator.maxTouchPoints > 1);
document.documentElement.classList.add(...(mobile ? ["mobile"] : ua.includes("Mac OS X") ? ["mac"] : []));

createApp(App).mount("#app");

// No page zoom: trackpad pinch arrives as ctrl+wheel (or gesture events in WebKit), and ctrl +/-/0 zooms too.
const stop = (e: Event) => e.preventDefault();
addEventListener("wheel", (e) => e.ctrlKey && e.preventDefault(), { passive: false });
for (const t of ["gesturestart", "gesturechange", "gestureend"]) addEventListener(t, stop);
addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && ["+", "=", "-", "_", "0"].includes(e.key)) e.preventDefault();
});
