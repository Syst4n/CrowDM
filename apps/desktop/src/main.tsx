import React from "react";
import ReactDOM from "react-dom/client";
import "./styles.css";
import { MainApp } from "./components/MainApp";
import { CaptureDialog } from "./components/CaptureDialog";

const isCapture = window.location.hash === "#capture";

// Block the WebView's own context menu and reload shortcuts outside inputs.
window.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement;
  if (!t.closest("input, textarea, .selectable")) e.preventDefault();
});
window.addEventListener("keydown", (e) => {
  if (e.key === "F5" || (e.ctrlKey && e.key.toLowerCase() === "r")) e.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{isCapture ? <CaptureDialog /> : <MainApp />}</React.StrictMode>,
);
