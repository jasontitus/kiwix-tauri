import { useEffect, useState } from "react";
import { api } from "../api";
import { t } from "../i18n";
import { errorText } from "./UI";

/** Live state of the offline-library agent connector (kiwix-mcp). */
type AgentConnectorStatus = {
  supported: boolean;
  running: boolean;
  relay: string | null;
  relayInstalled: boolean;
  /** The app runs inside a Flatpak sandbox; use `flatpak run --command=…`. */
  flatpak: boolean;
  flatpakId: string | null;
  /** The app runs from an AppImage mount; the relay path is temporary. */
  appImage: boolean;
  /** Set when this is not the default profile: the relay needs --data-dir. */
  dataDir: string | null;
  callsToday: number;
  lastUsed: number | null;
  error: string | null;
};

/**
 * Settings → Agent connector: lets local AI agents (MCP clients) search and
 * read this library through `kiwix-mcp`. Off by default, never in private
 * mode, read-only tools only.
 */
export function AgentConnectorSettings({
  enabled,
  privateMode,
  readOnly,
  onChange,
}: {
  enabled: boolean;
  privateMode: boolean;
  readOnly: boolean;
  onChange: (enabled: boolean) => void;
}) {
  const [status, setStatus] = useState<AgentConnectorStatus | null>(null);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    let alive = true;
    const load = () =>
      api<AgentConnectorStatus>("agent_connector_status", {})
        .then((value) => alive && setStatus(value))
        .catch(() => alive && setStatus(null));
    void load();
    // Activity and errors change while the page is open.
    const timer = window.setInterval(load, 5000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [enabled, privateMode, readOnly]);
  if (!status?.supported) return null;
  const args = status.dataDir ? ["--data-dir", status.dataDir] : [];
  const relayArgs = status.flatpak
    ? ["run", "--command=kiwix-mcp", status.flatpakId ?? "org.kiwix.workbench", ...args]
    : args;
  const relayCommand = status.flatpak ? "flatpak" : status.relay ?? "kiwix-mcp";
  const command = [relayCommand, ...relayArgs].map((part) => `"${part}"`).join(" ");
  const config = JSON.stringify(
    { mcpServers: { kiwix: { command: relayCommand, args: relayArgs } } },
    null,
    2,
  );
  async function copy() {
    try {
      await navigator.clipboard.writeText(config);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  }
  return (
    <div className="agent-connector">
      <label className="check-field">
        <input
          type="checkbox"
          checked={enabled && !privateMode && !readOnly}
          disabled={privateMode || readOnly}
          onChange={(event) => onChange(event.target.checked)}
        />
        <span>{t("Agent connector")}</span>
      </label>
      <p className="muted">
        {privateMode
          ? t("The agent connector is off in private reading mode.")
          : readOnly
            ? t("The agent connector is off in read-only mode.")
            : t(
                "Let AI assistants on this computer (MCP clients such as Claude, VS Code or Cursor) search and read your offline library. They can only read; nothing leaves this computer unless the assistant sends it.",
              )}
      </p>
      {enabled && !privateMode && !readOnly && (
        <div className="agent-connector-details">
          {status.error ? (
            <p role="alert">{errorText(status.error)}</p>
          ) : (
            <p className="muted" aria-live="polite">
              {status.lastUsed
                ? t("Used today: {count} requests", { count: status.callsToday })
                : t("Waiting for an assistant to connect.")}
            </p>
          )}
          {!status.relayInstalled && (
            <p role="alert">{t("kiwix-mcp is missing from the Kiwix folder. Reinstall Kiwix.")}</p>
          )}
          {status.appImage ? (
            <p role="alert">
              {t("AppImage mount paths change on each launch. Extract the AppImage once and use squashfs-root/usr/bin/kiwix-mcp in the configuration.")}
            </p>
          ) : status.relayInstalled ? (
            <>
              <p className="muted">{t("Add this server to your assistant's MCP configuration:")}</p>
              <pre className="agent-connector-config" aria-label={t("MCP configuration")}>{config}</pre>
              <button type="button" onClick={() => void copy()}>
                {copied ? t("Copied") : t("Copy configuration")}
              </button>
              <p className="muted">
                {t("Or, for Claude Code: claude mcp add kiwix -- {command}", { command })}
              </p>
            </>
          ) : null}
        </div>
      )}
    </div>
  );
}
