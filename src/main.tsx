import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { MiniViz } from "./MiniViz";

function Root() {
  // The mini visualizer window loads this same app with `#mini` in the URL.
  return window.location.hash === "#mini" ? <MiniViz /> : <App />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);