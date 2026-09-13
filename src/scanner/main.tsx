import React from "react";
import ReactDOM from "react-dom/client";
import { ScannerOverlay } from "./ScannerOverlay";
import "../styles/tokens.css";
import "../styles/lookupPopup.css";
import "./scanner.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ScannerOverlay />
  </React.StrictMode>,
);
