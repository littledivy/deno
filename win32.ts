// Windows platform bindings. `Deno.dock` maps the cross-platform calls onto
// the taskbar (overlay badge + FlashWindowEx); the dock-only operations
// (icon menu, activation-policy visibility) are macOS-only and report
// `false` here so the dashboard can show them as unsupported.

import type { DesktopPlatform } from "./macos.ts";

export function createPlatform(dock: Deno.Dock): DesktopPlatform {
  return {
    os: "windows",
    label: "Windows · taskbar overlay",
    setBadge: (text) => dock.setBadge(text === "" ? null : text),
    requestAttention: (critical) =>
      dock.bounce(critical ? "critical" : "informational"),
    // No equivalent of the macOS dock menu / activation policy on Windows.
    setAppMenu: () => false,
    setVisible: () => false,
    notificationDeniedHint: () =>
      "Windows toasts need a registered AppUserModelID." +
      " Re-enable at Settings → System → Notifications.",
  };
}
