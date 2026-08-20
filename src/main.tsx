import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ConfirmDialogProvider } from "./components/ui/ConfirmDialogProvider";
import { ErrorBoundary } from "./components/ui/ErrorBoundary";
import { installGlobalErrorLogging } from "./lib/log";
import "./styles.css";
import "./styles/lookupPopup.css";

// Before the first render, so a throw while mounting is recorded too.
installGlobalErrorLogging();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <ConfirmDialogProvider>
        <App />
      </ConfirmDialogProvider>
    </ErrorBoundary>
  </React.StrictMode>,
);
