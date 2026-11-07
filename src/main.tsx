import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import App from "./App";
import { Overlay } from "./overlay/Overlay";
import { blockNativeContextMenu } from "./components/ContextMenu";
import "./styles.css";

blockNativeContextMenu();

const isOverlay = getCurrentWebviewWindow().label === "overlay";
if (isOverlay) document.documentElement.classList.add("overlay-root");

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {isOverlay ? <Overlay /> : <App />}
  </StrictMode>,
);
