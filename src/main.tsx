import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { MiniPlayer } from "./MiniPlayer";
import { initTheme } from "./theme";

// Theme before the first render, for whichever document this is (issues #25,
// #32): the stored mode has to win over the OS guess the pre-paint script made,
// and a change made in the *other* window has to reach this one. The mini
// player is a separate window with its own stylesheet, and this is what keeps it
// off the OS preference.
initTheme();

function Root() {
  // The mini player window loads this same app with `#mini` in the URL.
  return window.location.hash === "#mini" ? <MiniPlayer /> : <App />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);