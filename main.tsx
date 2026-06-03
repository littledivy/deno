/** @jsxRuntime classic */
/** @jsx h */

// Desktop feature test. Server-side only: the JSX below renders to an HTML
// string in this process and is served to the desktop window — there is no
// client bundle and no client-side framework. All native access goes through
// the typed `Deno.BrowserWindow` / `Deno.dock` / `Deno.Tray` bindings, and
// every binding validates its arguments with zod.

// deno-lint-ignore no-import-prefix
import { z } from "jsr:@zod/zod@^4";
import type { DesktopPlatform } from "./macos.ts";

// ─── Server-side JSX ─────────────────────────────────────────────────────────
// Minimal string renderer. `h` returns `Html` (pre-rendered markup); text
// children are escaped, `Html`/`raw()` children are passed through verbatim.

class Html {
  constructor(readonly value: string) {}
}

function raw(value: string): Html {
  return new Html(value);
}

function escape(text: string): string {
  return text.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
}

type Child = Html | string | number | boolean | null | undefined | Child[];

function renderChild(child: Child): string {
  if (child == null || child === false || child === true) return "";
  if (child instanceof Html) return child.value;
  if (Array.isArray(child)) return child.map(renderChild).join("");
  return escape(String(child));
}

const VOID_TAGS = new Set([
  "area",
  "base",
  "br",
  "col",
  "hr",
  "img",
  "input",
  "link",
  "meta",
  "wbr",
]);

function renderAttrs(props: Record<string, unknown> | null): string {
  if (!props) return "";
  let out = "";
  for (const [key, value] of Object.entries(props)) {
    if (key === "children" || value == null || value === false) continue;
    const name = key === "className" ? "class" : key;
    if (value === true) {
      out += ` ${name}`;
    } else if (name === "style" && typeof value === "object") {
      const css = Object.entries(value as Record<string, string>)
        .map(([k, v]) => `${k}:${v}`)
        .join(";");
      out += ` style="${escape(css)}"`;
    } else {
      out += ` ${name}="${escape(String(value))}"`;
    }
  }
  return out;
}

type Component = (props: Record<string, unknown>) => Html;

function h(
  tag: string | Component,
  props: Record<string, unknown> | null,
  ...children: Child[]
): Html {
  if (typeof tag === "function") return tag({ ...props, children });
  const inner = children.map(renderChild).join("");
  if (VOID_TAGS.has(tag)) return raw(`<${tag}${renderAttrs(props)}>`);
  return raw(`<${tag}${renderAttrs(props)}>${inner}</${tag}>`);
}

declare global {
  namespace JSX {
    type Element = Html;
    interface ElementChildrenAttribute {
      children: Record<never, never>;
    }
    interface IntrinsicElements {
      [tag: string]: Record<string, unknown>;
    }
  }

  // The desktop type lib provides `Notification` and the `Navigator`
  // interface but not these globals, which the runtime still exposes.
  function alert(message?: unknown): void;
  function confirm(message?: string): boolean;
  function prompt(message?: string, defaultValue?: string): string | null;
  const navigator: Navigator;
}

// ─── App state ───────────────────────────────────────────────────────────────

interface TestResult {
  pass: boolean;
  detail: string;
}

const testResults = new Map<string, TestResult>();
const eventLog: Record<string, unknown>[] = [];
let eventSeq = 0;
let mouseMoveCount = 0;

function record(name: string, pass: boolean, detail = ""): void {
  testResults.set(name, { pass, detail });
}

function pushEvent(entry: Record<string, unknown>): void {
  entry.ts = Date.now();
  entry.seq = ++eventSeq;
  eventLog.push(entry);
  if (eventLog.length > 200) eventLog.shift();
}

// ─── Platform ────────────────────────────────────────────────────────────────
// Load the matching platform module at runtime — no bundling, the unused
// module is never imported.

const platform: DesktopPlatform =
  await (Deno.build.os === "windows"
    ? import("./win32.ts")
    : import("./macos.ts")).then((m) => m.createPlatform(Deno.dock));

Deno.dock.addEventListener("menuclick", (e) => {
  pushEvent({ type: "dockmenuclick", id: e.detail.id });
});
Deno.dock.addEventListener("reopen", (e) => {
  pushEvent({
    type: "dockreopen",
    hasVisibleWindows: e.detail.hasVisibleWindows,
  });
});

// ─── Window ──────────────────────────────────────────────────────────────────

const win = new Deno.BrowserWindow({
  title: "Desktop Feature Test",
  width: 1100,
  height: 800,
  x: 50,
  y: 50,
  resizable: true,
  alwaysOnTop: false,
});

// ─── Sync auto tests ─────────────────────────────────────────────────────────

function check(
  name: string,
  fn: () => boolean | { ok: boolean; detail: string },
) {
  try {
    const r = fn();
    if (typeof r === "boolean") record(name, r, r ? "ok" : "");
    else record(name, r.ok, r.detail);
  } catch (e) {
    record(name, false, String(e));
  }
}

function runAutoTests(): void {
  check("windowId", () => {
    const id = win.windowId;
    return { ok: typeof id === "number" && id >= 0, detail: `${id}` };
  });
  check("resizable", () => {
    const before = win.isResizable();
    win.setResizable(false);
    const after = win.isResizable();
    win.setResizable(true);
    return { ok: before && !after, detail: `before=${before} after=${after}` };
  });
  check("alwaysOnTop", () => {
    const before = win.isAlwaysOnTop();
    win.setAlwaysOnTop(true);
    const after = win.isAlwaysOnTop();
    win.setAlwaysOnTop(false);
    return { ok: !before && after, detail: `before=${before} after=${after}` };
  });
  check("visibility", () => {
    const before = win.isVisible();
    win.hide();
    const hidden = win.isVisible();
    win.show();
    return {
      ok: before && !hidden,
      detail: `before=${before} hidden=${hidden}`,
    };
  });
  check("setTitle", () => {
    win.setTitle("Test Title Changed");
    win.setTitle("Desktop Feature Test");
    return true;
  });
  check("getSize", () => {
    const [w, h2] = win.getSize();
    return {
      ok: typeof w === "number" && typeof h2 === "number",
      detail: `[${w},${h2}]${w === 0 ? " (WEF cross-thread bug)" : ""}`,
    };
  });
  check("setSize", () => {
    win.setSize(1100, 800);
    return true;
  });
  check("getPosition", () => {
    const [x, y] = win.getPosition();
    return {
      ok: typeof x === "number" && typeof y === "number",
      detail: `[${x},${y}]${x === 0 ? " (WEF cross-thread bug)" : ""}`,
    };
  });
  check("setPosition", () => {
    win.setPosition(50, 50);
    return true;
  });
  check("focus", () => {
    win.focus();
    return true;
  });
  check("openDevtools", () => {
    // Second call must reuse the singleton DevTools window, not spawn another.
    win.openDevtools();
    win.openDevtools();
    return { ok: true, detail: "called twice ⇒ one window" };
  });
  check("isClosed", () => {
    const closed = win.isClosed();
    return { ok: !closed, detail: `live window reports closed=${closed}` };
  });
}

runAutoTests();

// ─── Async auto tests (executeJs) ────────────────────────────────────────────
// `executeJs` resolves with the evaluated value and rejects on error.

async function runAsyncTests(): Promise<void> {
  try {
    const v = await win.executeJs("1 + 1");
    record("executeJs:arithmetic", v === 2, `${v}`);
  } catch (e) {
    record("executeJs:arithmetic", false, String(e));
  }
  try {
    await win.executeJs("throw new Error('test error')");
    record("executeJs:error", false, "did not reject");
  } catch {
    record("executeJs:error", true, "rejected as expected");
  }
  try {
    const v = await win.executeJs("({ a: 1, b: [2, 3] })") as
      | { a: number; b: number[] }
      | null;
    record(
      "executeJs:complex",
      !!v && v.a === 1 && Array.isArray(v.b),
      JSON.stringify(v),
    );
  } catch (e) {
    record("executeJs:complex", false, String(e));
  }
  try {
    const v = await win.executeJs("document.title");
    record("executeJs:string", typeof v === "string", JSON.stringify(v));
  } catch (e) {
    record("executeJs:string", false, String(e));
  }
}

setTimeout(() => runAsyncTests(), 1500);

// ─── Menus ───────────────────────────────────────────────────────────────────

const contextMenu: Deno.MenuItem[] = [
  { item: { label: "Option A", id: "ctx-a", enabled: true } },
  { item: { label: "Option B", id: "ctx-b", enabled: true } },
  "separator",
  {
    submenu: {
      label: "More",
      items: [
        { item: { label: "Option C", id: "ctx-c", enabled: true } },
        { item: { label: "Option D", id: "ctx-d", enabled: true } },
      ],
    },
  },
];

const dockMenu: Deno.MenuItem[] = [
  { item: { label: "Dock Item A", id: "dock-a", enabled: true } },
  { item: { label: "Dock Item B", id: "dock-b", enabled: true } },
  "separator",
  { item: { label: "Disabled Item", id: "dock-disabled", enabled: false } },
];

const trayMenu: Deno.MenuItem[] = [
  { item: { label: "Tray Item A", id: "tray-a", enabled: true } },
  { item: { label: "Tray Item B", id: "tray-b", enabled: true } },
  "separator",
  { item: { label: "Quit", id: "tray-quit", enabled: true } },
];

win.setApplicationMenu([
  // On macOS the first submenu becomes the application menu.
  { submenu: { label: "App", items: [{ role: { role: "quit" } }] } },
  {
    submenu: {
      label: "File",
      items: [{
        item: {
          label: "Test Action",
          id: "test-action",
          accelerator: "CmdOrCtrl+T",
          enabled: true,
        },
      }],
    },
  },
  {
    submenu: {
      label: "Edit",
      items: [
        { role: { role: "copy" } },
        { role: { role: "paste" } },
        { role: { role: "cut" } },
      ],
    },
  },
  {
    submenu: {
      label: "Test",
      items: [
        { item: { label: "Action 1", id: "action-1", enabled: true } },
        { item: { label: "Action 2", id: "action-2", enabled: true } },
        "separator",
        { item: { label: "Disabled Item", id: "disabled", enabled: false } },
      ],
    },
  },
]);

// ─── Tray ────────────────────────────────────────────────────────────────────
// 16×16 transparent PNG, just enough to feed `Tray.setIcon()` without an asset.

const TRAY_ICON_PNG = new Uint8Array([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a,
  0x00,
  0x00,
  0x00,
  0x0d,
  0x49,
  0x48,
  0x44,
  0x52,
  0x00,
  0x00,
  0x00,
  0x10,
  0x00,
  0x00,
  0x00,
  0x10,
  0x08,
  0x06,
  0x00,
  0x00,
  0x00,
  0x1f,
  0xf3,
  0xff,
  0x61,
  0x00,
  0x00,
  0x00,
  0x1f,
  0x49,
  0x44,
  0x41,
  0x54,
  0x38,
  0x8d,
  0x63,
  0xfc,
  0xff,
  0xff,
  0x3f,
  0x03,
  0x35,
  0x00,
  0x20,
  0x80,
  0x98,
  0x18,
  0x44,
  0x71,
  0x10,
  0xc5,
  0x41,
  0x14,
  0x07,
  0x51,
  0x1c,
  0x44,
  0x71,
  0x10,
  0x00,
  0x00,
  0xb6,
  0xfd,
  0x00,
  0x01,
  0x4f,
  0x88,
  0xa1,
  0xc7,
  0x00,
  0x00,
  0x00,
  0x00,
  0x49,
  0x45,
  0x4e,
  0x44,
  0xae,
  0x42,
  0x60,
  0x82,
]);

let activeTray: Deno.Tray | null = null;

// ─── Notifications ───────────────────────────────────────────────────────────

const activeNotifications = new Map<number, Notification>();
let notificationCounter = 0;

// ─── Secondary window / home url ─────────────────────────────────────────────

let secondWin: Deno.BrowserWindow | null = null;
let savedHomeUrl: string | null = null;

// ─── Bindings ────────────────────────────────────────────────────────────────
// `bind` parses the JS-side arguments with a zod tuple schema before handing
// them to the typed handler, so a malformed call from the page fails loudly.

// Arguments are validated with the zod `schema`; the return value is any
// structured-clone-able value (the runtime serializes it on the way out).
type BindFn = Deno.WindowBindings[string];
type BindValue = Awaited<ReturnType<BindFn>>;

function bind<S extends z.ZodTypeAny>(
  name: string,
  schema: S,
  handler: (args: z.infer<S>) => unknown,
): void {
  win.bind(
    name,
    async (...args) => await handler(schema.parse(args)) as BindValue,
  );
}

const Any = z.any();
const NoArgs = z.tuple([]);
const OneString = z.tuple([z.coerce.string()]);

bind(
  "echo",
  z.tuple([Any]).rest(Any),
  (args) => Promise.resolve(args.length === 1 ? args[0] : args),
);
bind(
  "add",
  z.tuple([z.number(), z.number()]),
  ([a, b]) => Promise.resolve(a + b),
);

bind(
  "getTestResults",
  NoArgs,
  () => Promise.resolve(Object.fromEntries(testResults)),
);
bind("getEventLog", NoArgs, () => Promise.resolve(eventLog.slice(-100)));
bind("triggerAutoTests", NoArgs, async () => {
  runAutoTests();
  await runAsyncTests();
  return Object.fromEntries(testResults);
});

bind("triggerContextMenu", z.tuple([z.number(), z.number()]), ([x, y]) => {
  win.showContextMenu(x, y, contextMenu);
  return Promise.resolve(null);
});

bind("showAlert", OneString, ([msg]) => {
  alert(msg);
  return Promise.resolve("done");
});
bind("showConfirm", OneString, ([msg]) => Promise.resolve(confirm(msg)));
bind(
  "showPrompt",
  z.tuple([z.coerce.string(), z.coerce.string().nullish()]),
  ([msg, def]) => Promise.resolve(prompt(msg, def ?? undefined)),
);

// Dock / taskbar (via the platform abstraction).
bind("dockSetBadge", OneString, ([text]) => {
  platform.setBadge(text);
  return Promise.resolve(true);
});
bind("dockClearBadge", NoArgs, () => {
  platform.setBadge("");
  return Promise.resolve(true);
});
bind("dockBounce", z.tuple([z.boolean().default(false)]), ([critical]) => {
  platform.requestAttention(critical);
  return Promise.resolve(true);
});
bind(
  "dockSetMenu",
  NoArgs,
  () => Promise.resolve(platform.setAppMenu(dockMenu)),
);
bind("dockHide", NoArgs, () => Promise.resolve(platform.setVisible(false)));
bind("dockShow", NoArgs, () => Promise.resolve(platform.setVisible(true)));

// Tray.
bind("trayCreate", NoArgs, () => {
  if (activeTray) {
    return Promise.resolve({
      ok: true,
      trayId: activeTray.trayId,
      reused: true,
    });
  }
  const tray = new Deno.Tray();
  tray.setIcon(TRAY_ICON_PNG);
  tray.setTooltip("Desktop Feature Test");
  tray.setMenu(trayMenu);
  tray.addEventListener(
    "click",
    () => pushEvent({ type: "trayclick", trayId: tray.trayId }),
  );
  tray.addEventListener(
    "dblclick",
    () => pushEvent({ type: "traydblclick", trayId: tray.trayId }),
  );
  tray.addEventListener(
    "menuclick",
    (e) =>
      pushEvent({
        type: "traymenuclick",
        trayId: tray.trayId,
        id: e.detail.id,
      }),
  );
  activeTray = tray;
  return Promise.resolve({ ok: true, trayId: tray.trayId, reused: false });
});
bind("trayDestroy", NoArgs, () => {
  if (!activeTray) {
    return Promise.resolve({ ok: false, reason: "no active tray" });
  }
  activeTray.destroy();
  activeTray = null;
  return Promise.resolve({ ok: true });
});
bind("traySetTooltip", OneString, ([text]) => {
  if (!activeTray) {
    return Promise.resolve({ ok: false, reason: "no active tray" });
  }
  activeTray.setTooltip(text);
  return Promise.resolve({ ok: true });
});

// Notifications (Web Notifications API, routed to the native backend).
bind("notificationPermission", NoArgs, async () => {
  const status = await navigator.permissions.query({ name: "notifications" });
  return {
    cached: Notification.permission,
    permissionsApi: status.state,
    hint: status.state === "denied" ? platform.notificationDeniedHint() : "",
  };
});
bind("notificationRequestPermission", NoArgs, async () => {
  try {
    return { ok: true, permission: await Notification.requestPermission() };
  } catch (e) {
    return { ok: false, error: String(e) };
  }
});

const NotifOpts = z.tuple([
  z.object({
    title: z.string().default("Desktop Feature Test"),
    body: z.string().default("Notification body text"),
    tag: z.string().optional(),
    requireInteraction: z.boolean().default(false),
    silent: z.boolean().default(false),
    icon: z.string().optional(),
  }).prefault({}),
]);

bind("notificationShow", NotifOpts, ([opts]) => {
  try {
    const n = new Notification(opts.title, {
      body: opts.body,
      tag: opts.tag,
      requireInteraction: opts.requireInteraction,
      silent: opts.silent,
      icon: opts.icon,
    });
    const localId = ++notificationCounter;
    activeNotifications.set(localId, n);
    for (const type of ["show", "click", "close", "error"] as const) {
      n.addEventListener(type, () => {
        pushEvent({ type: `notification:${type}`, localId, title: n.title });
        if (type === "close" || type === "error") {
          activeNotifications.delete(localId);
        }
      });
    }
    return Promise.resolve({
      ok: true,
      localId,
      title: n.title,
      permission: Notification.permission,
    });
  } catch (e) {
    return Promise.resolve({ ok: false, error: String(e) });
  }
});
bind("notificationCloseLast", NoArgs, () => {
  const keys = [...activeNotifications.keys()];
  if (keys.length === 0) {
    return Promise.resolve({ ok: false, reason: "no active notification" });
  }
  const id = keys[keys.length - 1];
  activeNotifications.get(id)!.close();
  activeNotifications.delete(id);
  return Promise.resolve({ ok: true, localId: id });
});
bind("notificationCloseAll", NoArgs, () => {
  const closed = activeNotifications.size;
  for (const n of activeNotifications.values()) n.close();
  activeNotifications.clear();
  return Promise.resolve({ ok: true, closed });
});

// Secondary window (multi-window bookkeeping).
bind("secondWindowOpen", NoArgs, async () => {
  if (secondWin && !secondWin.isClosed()) {
    secondWin.focus();
    return { ok: true, reused: true, windowId: secondWin.windowId };
  }
  secondWin = new Deno.BrowserWindow({
    title: "Secondary Window",
    width: 600,
    height: 400,
    x: 200,
    y: 200,
  });
  secondWin.addEventListener(
    "close",
    () => pushEvent({ type: "secondwin:close", windowId: secondWin?.windowId }),
  );
  await new Promise((r) => setTimeout(r, 200));
  await secondWin.executeJs(
    "document.body.style = 'background:#222;color:#fff;font-family:sans-serif;padding:20px';" +
      "document.body.textContent = 'Secondary window — close me to test isClosed()'",
  );
  return { ok: true, reused: false, windowId: secondWin.windowId };
});
bind("secondWindowClose", NoArgs, () => {
  if (!secondWin) {
    return Promise.resolve({ ok: false, reason: "no second window" });
  }
  secondWin.close();
  return Promise.resolve({ ok: true });
});
bind("secondWindowStatus", NoArgs, () =>
  Promise.resolve(
    secondWin
      ? {
        exists: true,
        windowId: secondWin.windowId,
        isClosed: secondWin.isClosed(),
      }
      : { exists: false },
  ));

// Navigate / reload.
bind("windowReload", NoArgs, () => {
  win.reload();
  return Promise.resolve(true);
});
bind("windowNavigate", OneString, ([url]) => {
  win.navigate(url);
  return Promise.resolve(true);
});
bind("windowSetHome", OneString, ([url]) => {
  savedHomeUrl = url;
  return Promise.resolve(true);
});
bind("windowGoHome", NoArgs, () => {
  if (!savedHomeUrl) {
    return Promise.resolve({ ok: false, reason: "no home url stashed" });
  }
  win.navigate(savedHomeUrl);
  return Promise.resolve({ ok: true });
});

// ─── Window event listeners ──────────────────────────────────────────────────

for (const type of ["keydown", "keyup"] as const) {
  win.addEventListener(type, (e) =>
    pushEvent({
      type,
      key: e.key,
      code: e.code,
      ctrl: e.ctrlKey,
      shift: e.shiftKey,
      alt: e.altKey,
      meta: e.metaKey,
      repeat: e.repeat,
    }));
}
for (const type of ["mousedown", "mouseup", "click", "dblclick"] as const) {
  win.addEventListener(
    type,
    (e) => pushEvent({ type, button: e.button, x: e.clientX, y: e.clientY }),
  );
}
win.addEventListener("mousedown", (e) => {
  // WEF may not forward `contextmenu` to the webview, so drive it from here.
  if (e.button === 2) win.showContextMenu(e.clientX, e.clientY, contextMenu);
});
win.addEventListener("mousemove", (e) => {
  if (++mouseMoveCount % 20 === 0) {
    pushEvent({ type: "mousemove", x: e.clientX, y: e.clientY });
  }
});
for (const type of ["mouseenter", "mouseleave", "focus", "blur"] as const) {
  win.addEventListener(type, () => pushEvent({ type }));
}
win.addEventListener(
  "wheel",
  (e) =>
    pushEvent({
      type: "wheel",
      deltaX: e.deltaX,
      deltaY: e.deltaY,
      deltaMode: e.deltaMode,
    }),
);
win.addEventListener(
  "resize",
  (e) =>
    pushEvent({
      type: "resize",
      width: e.detail.width,
      height: e.detail.height,
    }),
);
win.addEventListener(
  "move",
  (e) => pushEvent({ type: "move", x: e.detail.x, y: e.detail.y }),
);
win.addEventListener(
  "menuclick",
  (e) => pushEvent({ type: "menuclick", id: e.detail.id }),
);
win.addEventListener(
  "contextmenuclick",
  (e) => pushEvent({ type: "contextmenuclick", id: e.detail.id }),
);

// ─── Page (server-rendered JSX) ──────────────────────────────────────────────

function Card(
  props: { title: string; count?: string; full?: boolean; children?: Child },
): Html {
  return (
    <div class={props.full ? "card full" : "card"}>
      <h2>
        {props.title}
        {props.count ? <span class="count" id={props.count}>0</span> : null}
      </h2>
      {props.children}
    </div>
  );
}

function Log(props: { id: string; children?: Child }): Html {
  return <div id={props.id} class="log">{props.children}</div>;
}

function Page(): Html {
  const version = Deno.desktopVersion ?? "dev";
  return (
    <html>
      <head>
        <meta charset="utf-8" />
        <title>Desktop Feature Test</title>
        <style>{raw(STYLES)}</style>
      </head>
      <body>
        <div class="header">
          <h1>Desktop Feature Test</h1>
          <div class="header-right">
            <span class="platform">{platform.label} · v{version}</span>
            <button type="button" onclick="rerunAutoTests()">
              Re-run Auto Tests
            </button>
            <button type="button" onclick="generateReport()">
              Generate Report
            </button>
          </div>
        </div>

        <div class="grid">
          <Card title="Window Properties">
            <Log id="props-results">Loading...</Log>
          </Card>
          <Card title="executeJs">
            <Log id="exec-results">Loading...</Log>
          </Card>
          <Card title="Bindings Roundtrip">
            <Log id="bind-results">Running...</Log>
          </Card>

          <Card title="App Menu" count="menu-count">
            <p class="hint">
              Click menu items: File ▸ Test Action, Test ▸ Action 1/2
            </p>
            <Log id="menu-log">Waiting for menu clicks...</Log>
          </Card>
          <Card title="Keyboard" count="key-count">
            <p class="hint">Press any key</p>
            <Log id="key-log">Waiting for key events...</Log>
          </Card>
          <Card title="Mouse" count="mouse-count">
            <div class="test-area" id="mouse-area">
              Click / double-click here
            </div>
            <Log id="mouse-log">Waiting for mouse events...</Log>
          </Card>
          <Card title="Wheel" count="wheel-count">
            <p class="hint">Scroll anywhere in the window</p>
            <Log id="wheel-log">Waiting for wheel events...</Log>
          </Card>
          <Card title="Focus / Blur" count="focus-count">
            <p class="hint">Click outside the window, then back</p>
            <Log id="focus-log">Waiting for focus events...</Log>
          </Card>
          <Card title="Resize / Move" count="winev-count">
            <p class="hint">Resize or drag the window</p>
            <Log id="winev-log">Waiting for resize/move events...</Log>
          </Card>

          <Card title="Dialogs">
            <div class="btn-row">
              <button type="button" onclick="testAlert()">Alert</button>
              <button type="button" onclick="testConfirm()">Confirm</button>
              <button type="button" onclick="testPrompt()">Prompt</button>
            </div>
            <Log id="dialog-log">Click a button above</Log>
          </Card>
          <Card title="Context Menu" count="ctx-count">
            <div class="test-area" id="ctx-area" oncontextmenu="return false;">
              Right-click here
            </div>
            <Log id="ctx-log">Waiting for context menu clicks...</Log>
          </Card>

          <Card title="Dock" count="dock-count">
            <div class="btn-row">
              <input
                id="dock-badge-input"
                class="text-input"
                placeholder="badge text"
                value="3"
              />
              <button type="button" onclick="dockSetBadge()">Set Badge</button>
              <button type="button" onclick="dockClearBadge()">
                Clear Badge
              </button>
            </div>
            <div class="btn-row">
              <button type="button" onclick="dockBounce(false)">Bounce</button>
              <button type="button" onclick="dockBounce(true)">
                Bounce Critical
              </button>
              <button type="button" onclick="dockSetMenu()">
                Set Dock Menu
              </button>
            </div>
            <div class="btn-row">
              <button type="button" onclick="dockHide()">Hide Dock</button>
              <button type="button" onclick="dockShow()">Show Dock</button>
            </div>
            <Log id="dock-log">
              Dock menu / visibility are macOS-only; Windows reports
              unsupported.
            </Log>
          </Card>
          <Card title="Tray" count="tray-count">
            <div class="btn-row">
              <button type="button" onclick="trayCreate()">Create Tray</button>
              <button type="button" onclick="trayDestroy()">
                Destroy Tray
              </button>
              <button type="button" onclick="traySetTooltip()">
                Set Tooltip
              </button>
            </div>
            <Log id="tray-log">
              Click the tray icon (or its menu) once created.
            </Log>
          </Card>

          <Card title="Notifications" count="notif-count">
            <div class="btn-row">
              <button type="button" onclick="notifRequestPermission()">
                Request Permission
              </button>
              <button type="button" onclick="notifQueryPermission()">
                Query Permission
              </button>
              <span id="notif-perm" class="hint inline">perm: ?</span>
            </div>
            <div class="btn-row">
              <button type="button" onclick="notifShow()">Show</button>
              <button type="button" onclick="notifShowPersistent()">
                Show (requireInteraction)
              </button>
              <button type="button" onclick="notifShowSilent()">
                Show (silent)
              </button>
            </div>
            <div class="btn-row">
              <button type="button" onclick="notifShowTagged()">
                Show (tag="reuse")
              </button>
              <button type="button" onclick="notifCloseLast()">
                Close Last
              </button>
              <button type="button" onclick="notifCloseAll()">Close All</button>
            </div>
            <Log id="notif-log">
              Request permission first, then Show. Events:
              show/click/close/error.
            </Log>
          </Card>
          <Card title="Secondary Window" count="secondwin-count">
            <div class="btn-row">
              <button type="button" onclick="secondWindowOpen()">
                Open Window
              </button>
              <button type="button" onclick="secondWindowClose()">
                Close (op)
              </button>
              <button type="button" onclick="secondWindowStatus()">
                Status
              </button>
            </div>
            <Log id="secondwin-log">
              Tests multi-window tracking + isClosed().
            </Log>
          </Card>
          <Card title="Navigate / Reload">
            <div class="btn-row">
              <button type="button" onclick="windowReload()">reload()</button>
              <button type="button" onclick="windowNavigateBlank()">
                navigate(blank page)
              </button>
              <button type="button" onclick="windowGoHome()">
                navigate(home)
              </button>
            </div>
            <Log id="navigate-log">
              reload() goes through location.reload(); navigate() replaces the
              document.
            </Log>
          </Card>
        </div>

        <div id="report" class="report" style={{ display: "none" }}></div>
        <script>{raw(CLIENT_JS)}</script>
      </body>
    </html>
  );
}

const STYLES = `
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: #1a1a2e; color: #e0e0e0; padding: 16px;
    font-size: 13px; line-height: 1.5;
  }
  h1 { font-size: 20px; margin-bottom: 4px; color: #fff; }
  h2 { font-size: 14px; margin-bottom: 8px; color: #aaa; border-bottom: 1px solid #333; padding-bottom: 4px; }
  .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; }
  .header-right { display: flex; gap: 8px; align-items: center; }
  .platform { color: #6c6; font-size: 11px; }
  .hint { color: #888; margin-bottom: 4px; }
  .hint.inline { align-self: center; margin-left: 6px; margin-bottom: 0; }
  button {
    background: #2d2d4a; border: 1px solid #444; color: #e0e0e0;
    padding: 4px 10px; border-radius: 4px; cursor: pointer; font-size: 12px;
  }
  button:hover { background: #3d3d5a; }
  .text-input { background: #0d1117; border: 1px solid #444; color: #e0e0e0; padding: 2px 6px; border-radius: 4px; width: 80px; font-size: 12px; }
  .grid { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 10px; }
  .card { background: #16213e; border: 1px solid #333; border-radius: 6px; padding: 10px; }
  .card.full { grid-column: 1 / -1; }
  .pass { color: #4caf50; } .pass::before { content: "PASS "; font-weight: bold; }
  .fail { color: #f44336; } .fail::before { content: "FAIL "; font-weight: bold; }
  .pending { color: #888; } .pending::before { content: "-- "; }
  .log { background: #0d1117; border-radius: 4px; padding: 6px; max-height: 120px; overflow-y: auto; font-family: monospace; font-size: 11px; white-space: pre-wrap; }
  .test-area {
    background: #0d1117; border: 2px dashed #444; border-radius: 6px;
    min-height: 60px; display: flex; align-items: center; justify-content: center;
    color: #666; cursor: crosshair; user-select: none; margin-bottom: 6px;
  }
  .count { display: inline-block; background: #333; border-radius: 10px; padding: 0 6px; font-size: 11px; margin-left: 6px; }
  .report { background: #0d1117; border-radius: 6px; padding: 12px; margin-top: 12px; font-family: monospace; font-size: 12px; white-space: pre-wrap; }
  .btn-row { display: flex; gap: 6px; margin-bottom: 6px; }
`;

// Client glue. Runs in the desktop webview; talks back to this process through
// the injected `bindings` object. Plain DOM — no framework, no bundle.
const CLIENT_JS = `
const bindResults = [];

async function runBindTests() {
  const tests = [
    { label: 'echo("hello")', fn: () => bindings.echo("hello"), expect: "hello" },
    { label: 'echo(null)', fn: () => bindings.echo(null), expect: null },
    { label: 'echo(true)', fn: () => bindings.echo(true), expect: true },
    { label: 'echo(42)', fn: () => bindings.echo(42), expect: 42 },
    { label: 'echo([1,"two"])', fn: () => bindings.echo([1, "two"]), expect: [1, "two"] },
    { label: 'echo({a:1})', fn: () => bindings.echo({ a: 1 }), expect: { a: 1 } },
    { label: 'add(2, 3)', fn: () => bindings.add(2, 3), expect: 5 },
  ];
  let out = "";
  for (const t of tests) {
    try {
      const result = await t.fn();
      const pass = JSON.stringify(result) === JSON.stringify(t.expect);
      out += (pass ? "PASS" : "FAIL") + " " + t.label + " => " + JSON.stringify(result) + "\\n";
      bindResults.push({ label: t.label, pass, result });
    } catch (e) {
      out += "FAIL " + t.label + " threw: " + e + "\\n";
      bindResults.push({ label: t.label, pass: false, error: String(e) });
    }
  }
  document.getElementById("bind-results").textContent = out;
}

async function renderAutoResults() {
  try {
    const results = await bindings.getTestResults();
    const propKeys = ["windowId", "resizable", "alwaysOnTop", "visibility", "setTitle", "getSize", "setSize", "getPosition", "setPosition", "focus", "openDevtools", "isClosed"];
    const execKeys = ["executeJs:arithmetic", "executeJs:error", "executeJs:complex", "executeJs:string"];
    const fmt = (keys, strip) => keys.map(k => {
      const r = results[k];
      const label = strip ? k.replace("executeJs:", "") : k;
      if (!r) return '<span class="pending">' + label + "</span>";
      return '<span class="' + (r.pass ? "pass" : "fail") + '">' + label + "</span>  " + r.detail;
    }).join("\\n");
    document.getElementById("props-results").innerHTML = fmt(propKeys, false);
    document.getElementById("exec-results").innerHTML = fmt(execKeys, true);
  } catch (e) {
    document.getElementById("props-results").textContent = "Error: " + e;
  }
}

const eventCounts = {};
function updateCount(id, types) {
  document.getElementById(id).textContent = types.reduce((s, t) => s + (eventCounts[t] || 0), 0);
}
function renderEvents(elId, events, types) {
  const filtered = events.filter(e => types.includes(e.type)).slice(-8);
  if (filtered.length === 0) return;
  document.getElementById(elId).textContent = filtered.map(e => {
    const { type, ts, seq, ...rest } = e;
    return type + " " + JSON.stringify(rest);
  }).join("\\n");
}

let lastSeq = 0;
async function pollEvents() {
  try {
    const events = await bindings.getEventLog();
    const fresh = events.filter(e => e.seq > lastSeq);
    if (fresh.length === 0) return;
    lastSeq = events[events.length - 1].seq;
    for (const e of fresh) eventCounts[e.type] = (eventCounts[e.type] || 0) + 1;

    renderEvents("key-log", events, ["keydown", "keyup"]); updateCount("key-count", ["keydown", "keyup"]);
    renderEvents("mouse-log", events, ["mousedown", "mouseup", "click", "dblclick", "mousemove", "mouseenter", "mouseleave"]); updateCount("mouse-count", ["mousedown", "mouseup", "click", "dblclick", "mousemove"]);
    renderEvents("wheel-log", events, ["wheel"]); updateCount("wheel-count", ["wheel"]);
    renderEvents("focus-log", events, ["focus", "blur"]); updateCount("focus-count", ["focus", "blur"]);
    renderEvents("winev-log", events, ["resize", "move"]); updateCount("winev-count", ["resize", "move"]);
    renderEvents("menu-log", events, ["menuclick"]); updateCount("menu-count", ["menuclick"]);
    renderEvents("ctx-log", events, ["contextmenuclick"]); updateCount("ctx-count", ["contextmenuclick"]);
    renderEvents("dock-log", events, ["dockmenuclick", "dockreopen"]); updateCount("dock-count", ["dockmenuclick", "dockreopen"]);
    renderEvents("tray-log", events, ["trayclick", "traydblclick", "traymenuclick"]); updateCount("tray-count", ["trayclick", "traydblclick", "traymenuclick"]);
    renderEvents("secondwin-log", events, ["secondwin:close"]); updateCount("secondwin-count", ["secondwin:close"]);
    renderEvents("notif-log", events, ["notification:show", "notification:click", "notification:close", "notification:error"]); updateCount("notif-count", ["notification:show", "notification:click", "notification:close", "notification:error"]);
  } catch (_) {}
}

document.getElementById("ctx-area").addEventListener("contextmenu", async (e) => {
  e.preventDefault();
  await bindings.triggerContextMenu(e.clientX, e.clientY);
});

async function testAlert() {
  document.getElementById("dialog-log").textContent = "Showing alert...";
  await bindings.showAlert("Test alert message");
  document.getElementById("dialog-log").textContent = "alert() completed (dismissed by user)";
}
async function testConfirm() {
  const r = await bindings.showConfirm("Do you confirm?");
  document.getElementById("dialog-log").textContent = "confirm() returned: " + JSON.stringify(r);
}
async function testPrompt() {
  const r = await bindings.showPrompt("Enter something:", "default value");
  document.getElementById("dialog-log").textContent = "prompt() returned: " + JSON.stringify(r);
}

async function dockSetBadge() {
  const text = document.getElementById("dock-badge-input").value || "";
  await bindings.dockSetBadge(text);
  document.getElementById("dock-log").textContent = "setBadge(" + JSON.stringify(text) + ") OK";
}
async function dockClearBadge() { await bindings.dockClearBadge(); document.getElementById("dock-log").textContent = "setBadge('') OK"; }
async function dockBounce(critical) { await bindings.dockBounce(critical); document.getElementById("dock-log").textContent = "requestAttention(critical=" + critical + ") OK"; }
async function dockSetMenu() { const ok = await bindings.dockSetMenu(); document.getElementById("dock-log").textContent = ok ? "setMenu OK — right-click the dock icon" : "dock menu unsupported on this platform"; }
async function dockHide() { const ok = await bindings.dockHide(); document.getElementById("dock-log").textContent = ok ? "setVisible(false) OK" : "visibility unsupported on this platform"; }
async function dockShow() { const ok = await bindings.dockShow(); document.getElementById("dock-log").textContent = ok ? "setVisible(true) OK" : "visibility unsupported on this platform"; }

async function trayCreate() { document.getElementById("tray-log").textContent = "trayCreate => " + JSON.stringify(await bindings.trayCreate()); }
async function trayDestroy() { document.getElementById("tray-log").textContent = "trayDestroy => " + JSON.stringify(await bindings.trayDestroy()); }
async function traySetTooltip() {
  const text = "Tray @ " + new Date().toLocaleTimeString();
  document.getElementById("tray-log").textContent = "setTooltip(" + JSON.stringify(text) + ") => " + JSON.stringify(await bindings.traySetTooltip(text));
}

function setNotifPerm(p) { document.getElementById("notif-perm").textContent = "perm: " + p; }
async function notifQueryPermission() {
  const p = await bindings.notificationPermission();
  setNotifPerm(p.permissionsApi);
  document.getElementById("notif-log").textContent =
    "Notification.permission (cached) => " + p.cached + "\\n" +
    "navigator.permissions.query => " + p.permissionsApi +
    (p.hint ? "\\n[hint] " + p.hint : "");
}
async function notifRequestPermission() {
  document.getElementById("notif-log").textContent = "Requesting permission...";
  const r = await bindings.notificationRequestPermission();
  if (r.ok) setNotifPerm(r.permission);
  document.getElementById("notif-log").textContent = "requestPermission => " + JSON.stringify(r);
}
async function showNotif(opts, label) {
  const r = await bindings.notificationShow(opts);
  if (r.ok && r.permission) setNotifPerm(r.permission);
  document.getElementById("notif-log").textContent = label + " => " + JSON.stringify(r);
}
const notifShow = () => showNotif({ title: "Desktop Feature Test", body: "Hello @ " + new Date().toLocaleTimeString() }, "show");
const notifShowPersistent = () => showNotif({ title: "Persistent Notification", body: "requireInteraction=true — won't auto-dismiss", requireInteraction: true }, "show(persistent)");
const notifShowSilent = () => showNotif({ title: "Silent Notification", body: "silent=true — no sound", silent: true }, "show(silent)");
async function notifShowTagged() {
  const r1 = await bindings.notificationShow({ title: "Tagged #1", body: "tag='reuse' — should be replaced", tag: "reuse" });
  await new Promise(r => setTimeout(r, 600));
  const r2 = await bindings.notificationShow({ title: "Tagged #2", body: "tag='reuse' — replaces #1 where honored", tag: "reuse" });
  if (r2.ok && r2.permission) setNotifPerm(r2.permission);
  document.getElementById("notif-log").textContent = "show(tag) #1 => " + JSON.stringify(r1) + "\\n" + "show(tag) #2 => " + JSON.stringify(r2);
}
async function notifCloseLast() { document.getElementById("notif-log").textContent = "closeLast => " + JSON.stringify(await bindings.notificationCloseLast()); }
async function notifCloseAll() { document.getElementById("notif-log").textContent = "closeAll => " + JSON.stringify(await bindings.notificationCloseAll()); }

async function secondWindowOpen() { document.getElementById("secondwin-log").textContent = "open => " + JSON.stringify(await bindings.secondWindowOpen()); }
async function secondWindowClose() { document.getElementById("secondwin-log").textContent = "close => " + JSON.stringify(await bindings.secondWindowClose()); }
async function secondWindowStatus() { document.getElementById("secondwin-log").textContent = "status => " + JSON.stringify(await bindings.secondWindowStatus()); }

bindings.windowSetHome(location.href);
async function windowReload() { document.getElementById("navigate-log").textContent = "reload() — page about to reload…"; await bindings.windowReload(); }
async function windowNavigateBlank() {
  document.getElementById("navigate-log").textContent = "navigate(data:…) — page about to leave…";
  await bindings.windowNavigate("data:text/html,<body style='background:%23222;color:%23ccc;font-family:sans-serif;padding:20px'><h1>Blank page (navigate test)</h1><p>Use the window's back gesture or reopen the dashboard.</p></body>");
}
async function windowGoHome() { document.getElementById("navigate-log").textContent = "navigate(home) => " + JSON.stringify(await bindings.windowGoHome()); }

async function rerunAutoTests() {
  document.getElementById("props-results").textContent = "Re-running...";
  document.getElementById("exec-results").textContent = "Re-running...";
  await bindings.triggerAutoTests();
  await renderAutoResults();
}

async function generateReport() {
  const results = await bindings.getTestResults();
  const el = document.getElementById("report");
  el.style.display = "block";
  const allTests = Object.entries(results);
  const passed = allTests.filter(([, r]) => r.pass);
  const failed = allTests.filter(([, r]) => !r.pass);
  const bindPassed = bindResults.filter(r => r.pass).length;
  const bindFailed = bindResults.filter(r => !r.pass).length;
  const observedEvents = Object.keys(eventCounts);

  let report = "=== DESKTOP FEATURE TEST REPORT ===\\n";
  report += "Generated: " + new Date().toISOString() + "\\n\\n";
  report += "--- Backend Auto Tests ---\\n";
  report += "Total: " + allTests.length + "  Passed: " + passed.length + "  Failed: " + failed.length + "\\n\\n";
  for (const [name, r] of allTests) report += (r.pass ? "  PASS" : "  FAIL") + "  " + name + "  " + r.detail + "\\n";
  report += "\\n--- Binding Roundtrip Tests ---\\n";
  report += "Total: " + bindResults.length + "  Passed: " + bindPassed + "  Failed: " + bindFailed + "\\n\\n";
  for (const r of bindResults) {
    report += (r.pass ? "  PASS" : "  FAIL") + "  " + r.label;
    if (!r.pass) report += r.error ? "  threw: " + r.error : "  got: " + JSON.stringify(r.result);
    report += "\\n";
  }
  report += "\\n--- Events Observed ---\\n";
  if (observedEvents.length === 0) report += "  (none yet -- interact with the window)\\n";
  else for (const t of observedEvents.sort()) report += "  " + t + ": " + eventCounts[t] + "\\n";
  report += "\\n--- Known Issues ---\\n  getSize/getPosition may return [0,0] (WEF cross-thread bug)\\n";
  const totalPassed = passed.length + bindPassed;
  const total = totalPassed + failed.length + bindFailed;
  report += "\\n=== SUMMARY: " + totalPassed + "/" + total + " tests passed";
  if (failed.length + bindFailed > 0) report += ", " + (failed.length + bindFailed) + " failed";
  report += " ===\\n";
  el.textContent = report;
  el.scrollIntoView({ behavior: "smooth" });
}

(async () => {
  await new Promise(r => setTimeout(r, 500));
  await runBindTests();
  await new Promise(r => setTimeout(r, 2500));
  await renderAutoResults();
  setInterval(pollEvents, 1500);
})();
`;

// ─── Render + serve ──────────────────────────────────────────────────────────
// The JSX is rendered to an HTML string here (server-side) and served over the
// loopback port the desktop runtime auto-binds the main window to — the window
// loads it with no port to pick. The listening socket also keeps the runtime
// alive. State is read back through the bindings, not server endpoints.

const page = "<!DOCTYPE html>" + renderChild(<Page />);

Deno.serve(() =>
  new Response(page, { headers: { "content-type": "text/html" } })
);
