import type { ReactNode } from "react";
import { CloseIcon } from "./icons";

export type Tone = "success" | "warning" | "danger" | "info" | "neutral";

const TONE: Record<Tone, { box: string; dot: string; ring: string }> = {
  success: { box: "bg-success-bg text-success", dot: "bg-success", ring: "rgb(var(--success-bg))" },
  warning: { box: "bg-warning-bg text-warning", dot: "bg-warning", ring: "rgb(var(--warning-bg))" },
  danger: { box: "bg-danger-bg text-danger", dot: "bg-danger", ring: "rgb(var(--danger-bg))" },
  info: { box: "bg-info-bg text-info", dot: "bg-info", ring: "rgb(var(--info-bg))" },
  neutral: { box: "bg-neutral-bg text-neutral", dot: "bg-neutral", ring: "rgb(var(--muted))" },
};

/** Status label: 26px, colored dot + text. Never color alone, always a word. */
export function Badge({ tone = "neutral", children, dot = true }: { tone?: Tone; children: ReactNode; dot?: boolean }) {
  return (
    <span className={`inline-flex h-[26px] items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 text-13 font-semibold ${TONE[tone].box}`}>
      {dot && <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${TONE[tone].dot}`} aria-hidden="true" />}
      {children}
    </span>
  );
}

// Labels from Buttons.dc.html "Status labels". Never color alone, always a word.
const STATUS: Record<"process" | "deploy" | "service", Record<string, [string, Tone]>> = {
  process: {
    running: ["Running", "success"],
    deploying: ["Deploying", "info"],
    crashed: ["Crashed", "danger"],
    failed: ["Crashed", "danger"],
    stopped: ["Stopped", "neutral"],
    inactive: ["Stopped", "neutral"],
  },
  deploy: {
    queued: ["Queued", "neutral"],
    running: ["Running", "info"],
    success: ["Success", "success"],
    failed: ["Failed", "danger"],
    cancelled: ["Cancelled", "neutral"],
    rolled_back: ["Rolled back", "warning"],
  },
  service: {
    active: ["Active", "success"],
    activating: ["Starting", "info"],
    reloading: ["Reloading", "info"],
    inactive: ["Inactive", "danger"],
    failed: ["Failed", "danger"],
  },
};

/**
 * API status string → label + tone.
 * kind="process" (default): systemd unit state of an app (running / deploying / crashed / stopped).
 * kind="deploy": deploys.status. kind="service": `systemctl is-active` of nginx, postgresql, …
 */
export function StatusBadge({ status, kind = "process" }: { status: string | null | undefined; kind?: keyof typeof STATUS }) {
  const key = (status ?? "").toLowerCase();
  const [label, tone] = STATUS[kind][key] ?? [status || "Unknown", "neutral" as Tone];
  return <Badge tone={tone}>{label}</Badge>;
}

/** HTTP status code (2xx green, 3xx blue, 4xx amber, 5xx red) or method (GET neutral, writes blue). Mono, 24px. */
export function HttpBadge({ value }: { value: string | number }) {
  const v = String(value).toUpperCase();
  const n = Number(v);
  let tone: Tone = "neutral";
  if (!Number.isNaN(n)) tone = n >= 500 ? "danger" : n >= 400 ? "warning" : n >= 300 ? "info" : "success";
  else if (v !== "GET" && v !== "HEAD" && v !== "OPTIONS") tone = "info";
  return (
    <span className={`inline-flex h-6 items-center rounded-md px-2 font-mono text-xs font-medium ${TONE[tone].box}`}>{v}</span>
  );
}

/**
 * Small data label, 24px, 12px 500.
 *   muted   – runtime / manager ("Rust · systemd") (default)
 *   outline – key/value, mono ("port 3010": pass prefix="port")
 *   solid   – mono, ink ground
 *   plain   – overflow count ("+2")
 */
export function Tag({
  children,
  variant = "muted",
  prefix,
  mono,
}: {
  children: ReactNode;
  variant?: "muted" | "outline" | "solid" | "plain";
  prefix?: ReactNode;
  mono?: boolean;
}) {
  const v = {
    muted: "bg-muted text-ink",
    outline: "border border-line-strong text-ink",
    solid: "bg-ink text-ink-on",
    plain: "text-ink-3",
  }[variant];
  const isMono = mono ?? (variant === "outline" || variant === "solid");
  return (
    <span className={`inline-flex h-6 items-center gap-1.5 whitespace-nowrap rounded-md px-2 text-xs font-medium ${isMono ? "font-mono" : ""} ${v}`}>
      {prefix != null && <span className="text-ink-3">{prefix}</span>}
      {children}
    </span>
  );
}

/** Removable filter chip, 32px pill, blue tint. */
export function Chip({ children, onRemove, removeLabel }: { children: ReactNode; onRemove: () => void; removeLabel?: string }) {
  return (
    <span className="inline-flex h-8 items-center gap-1 rounded-full bg-info-bg pl-3 pr-1 text-13 font-medium text-info-strong">
      {children}
      <button
        type="button"
        onClick={onRemove}
        aria-label={removeLabel ?? `Remove filter ${typeof children === "string" ? children : ""}`.trim()}
        className="inline-flex h-7 w-7 items-center justify-center rounded-full hover:bg-info-hover"
      >
        <CloseIcon size={14} strokeWidth={2.5} />
      </button>
    </span>
  );
}

/** Count pill. tone="brand" (red) only for things that need attention (crashed apps); "ink" for counts in buttons. */
export function CountBadge({ count, tone = "ink", label }: { count: number | string; tone?: "ink" | "brand"; label?: string }) {
  return (
    <span
      aria-label={label}
      className={`inline-flex h-5 min-w-5 items-center justify-center rounded-full px-1.5 font-mono text-xs font-semibold ${tone === "brand" ? "bg-brand text-white" : "bg-ink text-ink-on"}`}
    >
      {count}
    </span>
  );
}

/** Live indicator: 8px dot with a soft ring, plus text ("Server online"). */
export function LiveDot({ tone = "success", children }: { tone?: Tone; children?: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-2 text-13 font-medium">
      <span className={`h-2 w-2 shrink-0 rounded-full ${TONE[tone].dot}`} style={{ boxShadow: `0 0 0 3px ${TONE[tone].ring}` }} aria-hidden="true" />
      {children}
    </span>
  );
}
