import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import App from "./App";
import { Overlay } from "./overlay/Overlay";
import { blockNativeContextMenu } from "./components/ContextMenu";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { setLang } from "./i18n";
import "./styles.css";

blockNativeContextMenu();

const isOverlay = getCurrentWebviewWindow().label === "overlay";
if (isOverlay) document.documentElement.classList.add("overlay-root");

// El idioma se fija antes de dibujar; si cambia en Ajustes, la ventana se recarga.
api
  .settings()
  .then((s) => setLang(s.language))
  .catch(() => setLang("auto"))
  .finally(() => {
    createRoot(document.getElementById("root")!).render(<StrictMode>{isOverlay ? <Overlay /> : <App />}</StrictMode>);
  });
listen("language", () => location.reload());
