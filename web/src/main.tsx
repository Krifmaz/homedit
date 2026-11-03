/**
 * Entry point: picks the HTTP or mock API and mounts the app.
 *
 * Mock mode is on when built with `VITE_MOCK=1` or when the URL has `?mock`.
 */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HttpApi, readInlineData, setIconResolver, type Api } from "./api/client";
import { MockApi, mockIconUrl } from "./api/mock";
import { App } from "./app/App";
import { AppProvider } from "./app/context";
import "./styles.css";

const mock = import.meta.env.VITE_MOCK === "1" || new URLSearchParams(location.search).has("mock");
let api: Api;
if (mock) {
  const params = new URLSearchParams(location.search);
  const role = params.get("role") === "viewer" ? "viewer" : "admin";
  const auth = params.get("auth");
  api = new MockApi({
    role,
    latency: 150,
    auth: auth === "login" || auth === "setup" ? auth : "open",
  });
  setIconResolver(mockIconUrl);
} else {
  api = new HttpApi();
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <AppProvider api={api} initial={mock ? null : readInlineData()}>
      <App />
    </AppProvider>
  </StrictMode>,
);
