# Dashboard components

Design source: `tasks/server-manager/dashboard-design/*.dc.html` (copied from the Radio API console, same tokens). Copy is English (K6). Process manager is **systemd** everywhere, never PM2.
Everything is Tailwind with the tokens in `web/tailwind.config.js`. No purple, no gradients, shadows only on overlays.

## Tokens (Tailwind classes)

| Use | Class |
|---|---|
| App ground / card / table header | `bg-paper` / `bg-surface` / `bg-sunken` |
| Neutral fill (tags, skeleton, tab track) | `bg-muted` |
| Borders: card / input / row divider | `border-line` / `border-line-strong` / `border-line-soft` |
| Text: main / secondary / caption | `text-ink` / `text-ink-2` / `text-ink-3` |
| Sidebar ground and text | `bg-console`, `bg-console-raised`, `text-console-text|dim|muted` |
| Brand red (signal only: logo, live, active, attention count) | `bg-brand`, `text-brand-soft` |
| Links, focus, chart 1, deploying | `text-info`, `bg-info-bg`, `text-info-strong` |
| Terminal log lines | `text-term-cmd|ok|warn|err`, `bg-term-live` (on `bg-console`) |
| Chart 2 | `bg-chart-2` (#E9A15C) |
| Status fg / bg | `text-success bg-success-bg`, `text-warning bg-warning-bg`, `text-danger bg-danger-bg`, `text-info bg-info-bg`, `text-neutral bg-neutral-bg` |
| Type | `text-13` (13/18), `text-15`, `text-17` (section), `text-22`, `text-28` (page h1), `text-32`; `tracking-caps` for 12px uppercase labels; `font-mono` for ids, counts, codes |
| Radius | `rounded-md` 6 badge, `rounded-lg` 8 control, `rounded-xl` 12 card, `rounded-2xl` 16 modal |
| Shadow (overlays only) | `shadow-overlay` (menus), `shadow-modal`, `shadow-toast` |

Focus ring is global (`:focus-visible` 2px #1D5FBF, offset 2px) — don't add `focus:ring-*`.
Numbers: right-align and `font-mono` in tables, `tabular-nums` in big figures, format with `toLocaleString("en-US")`.

## Which Button variant

The variant follows **what the action does**, not how loud it should look. All buttons are 44px (`size="md"`).

| Variant | Use for | Examples |
|---|---|---|
| `primary` (ink) | Writes or runs | Deploy, Save, Create database, Restart & save |
| `secondary` (white, border) | Neutral | Cancel, Back, Restart, Try again |
| `subtle` (blue tint) | Reads or opens | View log, Copy, Open site |
| `danger` (red) | Destroys | Delete app, Drop database, Delete site |

One primary per view area. Busy: `busy` + the -ing label (`{saving ? "Saving…" : "Save changes"}`). Navigation that looks like a button: pass `href`. Inline text links are plain `<a>` (blue via `text-info`).

## Components

Imports are relative: `import { Button } from "../../components/Button"`.

### Button.tsx
- `Button` `{ variant?: primary|secondary|subtle|danger, size?: sm(36)|md(44)|lg(48), icon?, busy?, href?, ...button/anchor attrs }` — `type="button"` by default; `size="sm"` only inside dense table rows; `lg` for the single CTA of a form (Sign in).
- `IconButton` `{ label (required → aria-label + title), variant?: secondary|ghost|console, size?: sm|md, busy? }` — children is the icon.

### Badge.tsx
- `Badge` `{ tone: success|warning|danger|info|neutral, dot?=true }` — 26px status label.
- `StatusBadge` `{ status, kind?: "process"|"deploy"|"service" }` — process: running/deploying/crashed/stopped; deploy: queued/running/success/failed/cancelled/rolled_back; service (`systemctl is-active`): active/activating/inactive/failed.
- `HttpBadge` `{ value: 200 | "GET" }` — mono; 2xx green, 3xx blue, 4xx amber, 5xx red; GET neutral, writes blue.
- `Tag` `{ variant?: muted|outline|solid|plain, prefix?, mono? }` — runtime "Rust · systemd" (muted), `<Tag variant="outline" prefix="port">3010</Tag>`, "+2" (plain).
- `Chip` `{ onRemove, removeLabel? }` — removable active-filter chip.
- `CountBadge` `{ count, tone?: ink|brand, label? }` — count pill in a button or red attention count (crashed apps).
- `LiveDot` `{ tone }` — dot with ring + text ("Server online", "Deploying radio-api-rs").

### Input.tsx
All take `label`, `helper`, `error` (`true` = red without message), `optional`, `hideLabel`, `wrapperClassName`, plus native attrs; `className` goes on the control. Ids/aria wiring are automatic.
- `TextField` (alias `Input`) `{ mono? }` — 44px input.
- `PasswordField` — eye button toggles visibility.
- `Textarea` `{ rows?=3 }`.
- `Select` `{ options?: {value,label}[], placeholder? (adds value ""), children? }` — native select.
- `Toggle` `{ checked, onChange(bool), label, disabled? }` — role="switch", green when on.
- `Checkbox` `{ label, ...input attrs }`.

### SearchBar.tsx
- `SearchBar` `{ value, onChange, onSearch, onClear?, placeholder?, label?, buttonText?, busy? }` — role="search" form; Enter submits; clear (x) calls `onClear` or `onChange("")`.

### FilterTabs.tsx
- `FilterTabs` `{ options: {value,label,count?}[], value, onChange, label? }` — status filter above a list (All 5,093 · Synced · Never · Failed).
- `SegmentedControl` `{ options, value, onChange, label }` — view setting (24 h / 7 days / 30 days).

### Pagination.tsx (also re-exported from DataTable.tsx)
- `Pagination` `{ page (1-based), totalPages, onPageChange, total?, pageSize? }` — "21–40 of 106,797" + mono page buttons. Renders nothing when there is one page and no `total`.

### DataTable.tsx
- `DataTable<T>` `{ columns, data, loading?, skeletonRows?=5, emptyMessage?, empty?, rowHref?(item), onRowClick?(item), rowKey?, label?, framed?=true, minWidth?=640 }`
- `Column<T>` `{ key, header, render?, align?: left|right, mono?, width?, className? }`
- Sunken header with 12px caps labels, 52px rows. `rowHref` makes the first column a real link and the row clickable (clicks on inner buttons/links are ignored). Put it in `<Section flush>` with `framed={false}`, or use it standalone (framed).

### Card.tsx
- `Card` `{ padded?=true, className? }` — white card, 1px line.
- `Section` `{ title, subtitle?, action?, flush?, className? }` — card with h2 header; `flush` for tables/lists (no body padding, header rule).
- `KpiStrip` `{ items: {label, value, meta?, href?}[], loading? }` — one card split by rules; cells with `href` are links.
- `BarList` `{ items: {label, value, display?, href?}[], labelWidth?=96, color?: blue|amber, max? }` — label + bar + mono value.

### Feedback.tsx
- `Skeleton` `{ className, light? }` — size it like the content. `SkeletonBlock` — card-sized placeholder.
- `EmptyState` `{ title, description?, action?, icon? }` — say what is empty and how to get out of it; unframed.
- `ErrorState` `{ title, description?, onRetry?, retryLabel?, busy? }` — red inline alert + secondary "Try again".
- `InlineAlert` `{ title, children? }` — red role="alert" box (form errors).
- `Banner` `{ tone: info|warning|error|success, title?, children, action?: {label, href? | onClick?} }` — page-level message.

### Modal.tsx
- `Modal` `{ open, onClose, title, description?, children?, footer?, size?: sm|md(520)|lg|xl, initialFocus? }` — footer order: Cancel (secondary) … primary last.
- `ConfirmDialog` `{ open, onClose, onConfirm, title, children, confirmLabel, cancelLabel?, busy?, tone?: danger|primary, confirmText? }` — destructive confirm. Title is the question ("Delete radio-api-rs?"), body names the effect and the records (`<strong>` the record name), confirm label is verb + object ("Delete app", "Drop database"). `confirmText` = type-the-name gate (Overlays.dc.html). Focus starts on Cancel.
- `Drawer` `{ open, onClose, title, description?, children, footer?, width?=480 }` — right side panel.
- `useDialog(open, onClose, panelRef, initialFocusRef?)` — Esc, focus trap, scroll lock, focus return. Used by all three.

### Toast.tsx
- `ToastProvider` (already mounted in Layout) + `useToast()` → `toast({ title, body?, tone?: success|info|error, action?: {label, href? | onClick?} })`. Bottom-right, dark, auto-hides after 5 s. Confirms things that happened; an error that needs action belongs on the page (ErrorState / Banner), not only in a toast.

### PageHeader.tsx
- `PageHeader` `{ title, subtitle?, actions? }` — page h1 (28/36, 22 on mobile). The topbar breadcrumb shows section › page, so don't repeat the section in the title.

### Shell (Layout.tsx, Sidebar.tsx, Topbar.tsx)
- `Layout` — sidebar + topbar (lg) / mobile bar + drawer (below lg), `<main>` full width, padding 16/32. Pages render only their content; use `flex flex-col gap-6` (desktop design gap 24, list pages 20) as the page root.
- `usePageCrumb(label)` (from Layout.tsx) — detail pages set the last breadcrumb part: `usePageCrumb(app.name)` → "Apps › radio-api-rs" (default: the id from the URL).
- `NAV`, `crumbFor(route)` (Sidebar.tsx) — nav config; add new pages there.
- Topbar status pill pings `/api/auth/me` every 15 s ("Server online · ping 3 ms" / "Agent unreachable"). Phase 2 adds the warnings state.

### icons.tsx
Stroke icons `{ size?=18, strokeWidth?=2, className? }`: Plus, Refresh, Filter, Search, Close, Check, External, Menu, Logout, Edit, Trash, Copy, Chevron{Down,Right,Left}, Eye, EyeOff, AlertCircle, AlertTriangle, Info, EmptySearch, nav icons (Overview, Usage, Apps, Database, WebServer, Settings), Deploy, Restart, Branch, `DotsIcon` (filled), `ServerMarkIcon`, `Spinner`, `BrandTile`. Add new ones with the `icon("path d")` helper.

### Charts.tsx
`BarChart` built from div bars (last bar in ink), with a `labels` row or a custom `axis`, and `ariaLabel`.

### LogView.tsx
- `LogView` `{ title, lines: {text, t?, kind?: cmd|ok|warn|err}[], live?, height?=360 }` — dark terminal (DataDisplay.dc.html): Follow checkbox (auto-scroll), Copy. Kind is guessed from the text (`$ ` → cmd, `error` → err, `warning` → warn); stderr is not red because cargo/bun log progress there.

### Placeholder.tsx
- `Placeholder` `{ title, phase, screens }` — phase 1 stand-in page. Remove the usage when the page is built.

## lib/

- `api.ts` — `apiUrl(url)` turns `/api/...` into a page-relative `api/...` (the dashboard may run under a prefix such as `/server-manager/`; authFetch and useEventStream already use it, wrap any other fetch/EventSource URL), `authFetch(url, init)` (cookie auth, 401 → back to sign-in) and `api<T>(url, { method?, body?, signal? })` (JSON both ways, throws `ApiError` with the server's `error` text, `.status`, `.body`).
- `auth.tsx` — `AuthProvider`, `useAuth()` → `{ user, publicUrl, login, logout }` (`publicUrl` = SM_PUBLIC_URL or the request host + prefix; build webhook links from it).
- `router.ts` — `useHashRoute()`, `parseRoute(route)` → `{ segments, query }`, `navigate(path)`.
- `useEventStream.ts` — `useEventStream(url | null)` → `{ lines, done, connected }` for the SSE endpoints built with `src/sse.rs`.
- `server.ts` — `SERVER` (host / user / specs), `formatWib(unixSecs)`.
