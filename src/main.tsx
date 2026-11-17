import { Component, type ErrorInfo, type ReactNode, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { isNative } from "./api";
import { I18nProvider, t } from "./i18n";
import { applyPlatform } from "./platform";
import { reportBootPhase } from "./startup-ready";
import "./styles.css";
import "./platform.css";

reportBootPhase(
  isNative ? "module" : "module-browser",
  "Application module evaluated",
);

class AppBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(error, info.componentStack);
  }
  render() {
    if (this.state.error)
      return (
        <main className="startup-state">
          <h1>{t("The workbench could not be displayed")}</h1>
          <p role="alert">{this.state.error.message}</p>
          <button className="primary" onClick={() => window.location.reload()}>
            {t("Reload application")}
          </button>
        </main>
      );
    return this.props.children;
  }
}
// Keep Android app chrome at device scale; reader controls and map gestures
// scale archive content independently. Ordinary browser and iOS zoom stay intact.
if (isNative && import.meta.env.TAURI_ENV_PLATFORM === "android") {
  document.querySelector('meta[name="viewport"]')?.setAttribute(
    "content",
    "width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no",
  );
}
applyPlatform();
const root = document.getElementById("root");
if (!root) throw new Error("Missing application root");
reportBootPhase("rendering", "React root created");
createRoot(root).render(
  <StrictMode>
    <I18nProvider>
      <AppBoundary>
        <App />
      </AppBoundary>
    </I18nProvider>
  </StrictMode>,
);
