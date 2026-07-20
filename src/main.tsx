import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ConfirmDialogProvider } from "./components/ui/ConfirmDialogProvider";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ConfirmDialogProvider>
      <App />
    </ConfirmDialogProvider>
  </React.StrictMode>,
);
