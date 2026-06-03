// macOS platform bindings, backed by NSDockTile / NSApp via `Deno.dock`.

/** Platform-specific desktop surface. macOS and Windows differ in what the
 * app-icon (dock / taskbar) can do; the app talks to this interface and never
 * branches on `Deno.build.os` itself. */
export interface DesktopPlatform {
  readonly os: "darwin" | "windows";
  /** Human label shown in the dashboard status panel. */
  readonly label: string;
  /** Set the app-icon badge text. Empty string clears it. */
  setBadge(text: string): void;
  /** Demand user attention. `critical` bounces/flashes until focused. */
  requestAttention(critical: boolean): void;
  /** Install the app-icon right-click menu. Returns `false` if unsupported. */
  setAppMenu(items: Deno.MenuItem[]): boolean;
  /** Toggle app-icon visibility. Returns `false` if unsupported. */
  setVisible(visible: boolean): boolean;
  /** Hint shown when notification permission is denied. */
  notificationDeniedHint(): string;
}

export function createPlatform(dock: Deno.Dock): DesktopPlatform {
  return {
    os: "darwin",
    label: "macOS · NSDockTile / NSApp",
    setBadge: (text) => dock.setBadge(text === "" ? null : text),
    requestAttention: (critical) =>
      dock.bounce(critical ? "critical" : "informational"),
    setAppMenu: (items) => {
      dock.setMenu(items);
      return true;
    },
    setVisible: (visible) => {
      dock.setVisible(visible);
      return true;
    },
    notificationDeniedHint: () =>
      "macOS User Notifications cache denial per CFBundleIdentifier" +
      " (io.wef.cef for the CEF backend, io.wef.webview for webview)." +
      " Re-enable at System Settings → Notifications.",
  };
}
