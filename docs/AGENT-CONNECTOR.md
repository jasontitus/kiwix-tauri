# Agent connector (MCP)

The agent connector lets AI assistants on your computer search and read your offline Kiwix library. It works with Claude Desktop, Claude Code, VS Code, Cursor, or any other client of the [Model Context Protocol](https://modelcontextprotocol.io). Assistants can then answer from local Wikipedia, Stack Exchange or documentation archives with no Internet connection, and cite the article they used.

Availability: Windows, macOS and Linux desktop builds. The macOS app is sandboxed; its relay is bundled inside `Kiwix.app` and reaches the app through the container's private runtime socket.

## Turn it on

1. In Kiwix, open **Settings**, then turn on **Agent connector**. It is off by default and unavailable in private or read-only mode.
2. Add Kiwix to your assistant's MCP configuration. Settings shows the exact text to copy. For example:

   ```json
   { "mcpServers": { "kiwix": { "command": "C:\\Users\\you\\AppData\\Local\\Kiwix\\kiwix-mcp.exe", "args": [] } } }
   ```

   On macOS the bundled relay is inside the app:

   ```json
   { "mcpServers": { "kiwix": { "command": "/Applications/Kiwix.app/Contents/MacOS/kiwix-mcp", "args": [] } } }
   ```

   On Linux the packaged app places `kiwix-mcp` beside `kiwix-workbench`, for example `/usr/bin/kiwix-mcp`. For an AppImage, extract it once (`./Kiwix.AppImage --appimage-extract`) and use `squashfs-root/usr/bin/kiwix-mcp`; the temporary AppImage mount path changes on every launch.

   For Claude Code on Windows:

   ```
   claude mcp add kiwix -- "C:\Users\you\AppData\Local\Kiwix\kiwix-mcp.exe"
   ```

   If you run Kiwix with another profile (`--data-dir`, or a portable copy), Settings includes the matching `"--data-dir", "<folder>"` arguments. A portable `kiwix-mcp` finds its own profile when the portable marker is beside the relay; otherwise use the `--data-dir` arguments from Settings.

3. Keep Kiwix running while the assistant uses it. When Kiwix is closed, the assistant still sees the tools, and each call answers "Kiwix is not running". An assistant idle for more than five minutes is reconnected automatically on its next call.

## What an assistant can do

All three tools are read-only.

| Tool | What it does |
| --- | --- |
| `list_archives` | Lists the archives in your library: ID, title, language, article count and whether full-text search is available. |
| `search` | Searches titles and full text across all archives, or the ones named. Returns up to 20 results per page, with snippets and a `zim://` citation. |
| `read_article` | Returns one article as plain text, up to 40,000 characters per call; longer articles continue from the returned `nextOffset`. Kiwix reads at most the first 1 MiB of an article. |

An assistant cannot:
- change your library, settings or downloads;
- see your reading history, bookmarks or files outside the library;
- reach the network through Kiwix.

## Privacy and safety

- **Local only.** `kiwix-mcp` talks to Kiwix over a same-user IPC channel: a Windows named pipe, or a Unix domain socket on macOS and Linux. Only your own account can open it, and nothing listens on a network port.
- **Both ends check each other.**
  - Kiwix accepts a connection only from a process of your account.
  - On Windows, `kiwix-mcp` connects only to a pipe owned by your account and served by the `kiwix-workbench.exe` installed beside it.
  - On macOS and Linux, the socket lives in a `0700` directory and is `0600`; both ends verify the peer's uid. The macOS sandbox keeps the socket in the app container's private runtime directory.
- **Private and read-only modes.** Entering either mode closes the connector and cancels requests in progress, and it stays off until you leave that mode.
- **Untrusted text.** Archive content is written by others and may contain text aimed at AI assistants.
  - Kiwix removes text hidden by HTML attributes or inline styles, and invisible or reordering characters. Text hidden by a stylesheet class is not detected.
  - Every tool tells the assistant to treat results as reference material, not instructions.
  - This reduces the risk but cannot remove it. Review what an assistant does with what it read.
- **No local paths.** Errors returned to an assistant are fixed sentences; they never include file paths from your computer.
- **What reaches the assistant.** What you search for and read through the connector goes to your assistant. If the assistant uses a cloud model, the provider receives it, as with anything else you give that assistant.
- **Limits.**
  - At most two requests run at a time, and at most 60 a minute (bursts of 10).
  - Each request has a 10-second budget, and at most 16 assistants can be connected at once.
  - Queries are limited to 512 characters.
- **Activity.** Settings shows how many requests were made today. Kiwix does not log what was searched or read.

## Troubleshooting

| Message | What to do |
| --- | --- |
| "Kiwix is not running, or its agent connector is off" | Start Kiwix and turn on **Settings → Agent connector**. |
| "Refused a connection that did not come from Kiwix" | Another program answered on the connector's channel. Restart Kiwix; if it continues, sign out and in again. |
| "Another program already uses the agent connector's channel" | Shown in Settings when Kiwix cannot claim its channel. Close other Kiwix copies, or sign out and in again. |
| "The request took too long" | Pass `archiveIds` to search fewer archives. |
