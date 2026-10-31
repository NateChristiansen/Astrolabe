import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { version } from "../package.json";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <main>Astrolabe {version}</main>
  </StrictMode>,
);
