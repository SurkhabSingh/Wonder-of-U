import React from "react";
import ReactDOM from "react-dom/client";
import { ScannerOverlay } from "./ScannerOverlay";
import "../styles/tokens.css";
import "../styles/lookupPopup.css";
import "./scanner.css";

// The scanner overlay's own document. It shares the app's design tokens and the vendored
// dictionary stylesheet, but none of `styles.css` — nothing else in this window is an app
// screen, and pulling 3.5k lines of layout in for a subtitle line and a popup would be
// waste in a window that has to repaint over live video.
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ScannerOverlay />
  </React.StrictMode>,
);
