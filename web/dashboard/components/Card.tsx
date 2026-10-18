import { useId, type CSSProperties, type ReactNode } from "react";
import { Skeleton } from "./Feedback";

/** White surface, 1px line, 12px radius. No shadow (shadows are for overlays only). */
export function Card({ children, className = "", padded = true }: { children: ReactNode; className?: string; padded?: boolean }) {
  return <div className={`rounded-xl border border-line bg-surface ${padded ? "px-6 py-5" : "overflow-hidden"} ${className}`}>{children}</div>;
}

/**
 * Card with a heading row: title (17/24 600), optional subtitle, and an `action` slot on the right
 * (a link-style Button, a SegmentedControl, a big number…).
 * `flush` = body has no padding and the header gets a bottom rule: use it for tables and lists.
 */
export function Section({
  title,
  subtitle,
  action,
  children,
  flush,
  className = "",
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  flush?: boolean;
  className?: string;
}) {
  const id = useId();
  const header = (
    <div className={`flex flex-wrap items-start justify-between gap-3 ${flush ? "border-b border-line px-6 py-4" : ""}`}>
      <div className="min-w-0">
        <h2 id={id} className="text-17 font-semibold text-ink">
          {title}
        </h2>
        {subtitle && <div className="text-13 text-ink-2">{subtitle}</div>}
      </div>
      {action && <div className="flex shrink-0 items-center gap-2">{action}</div>}
    </div>
  );
  return (
    <section aria-labelledby={id} className={`rounded-xl border border-line bg-surface ${flush ? "overflow-hidden" : "flex flex-col gap-4 px-6 py-5"} ${className}`}>
      {header}
      {children}
    </section>
  );
}

export interface KpiItem {
  label: ReactNode;
  value: ReactNode;
  meta?: ReactNode;
  /** Makes the whole cell a link (e.g. "#/apps?status=crashed"). */
  href?: string;
}

/** One card split into equal cells by 1px rules. 2 per row on mobile (1 when the count is odd), all in one row from lg. */
export function KpiStrip({ items, loading }: { items: KpiItem[]; loading?: boolean }) {
  const n = items.length;
  return (
    <div
      style={{ "--kpi-cols": n } as CSSProperties}
      className={`grid gap-px overflow-hidden rounded-xl border border-line bg-line ${n % 2 === 0 ? "grid-cols-2" : "grid-cols-1"} lg:grid-cols-[repeat(var(--kpi-cols),minmax(0,1fr))]`}
    >
      {items.map((k, i) => {
        const body = (
          <>
            <div className="text-13 font-medium text-ink-2">{k.label}</div>
            {loading ? (
              <Skeleton className="my-1.5 h-6 w-3/5 rounded-md" />
            ) : (
              <div className="text-28 font-semibold tabular-nums text-ink">{k.value}</div>
            )}
            {k.meta != null && <div className="text-13 text-ink-3">{loading ? <Skeleton light className="mt-1 h-3 w-4/5" /> : k.meta}</div>}
          </>
        );
        const cls = "flex min-w-0 flex-col gap-0.5 bg-surface px-5 py-[18px]";
        return k.href ? (
          <a key={i} href={k.href} className={`${cls} text-ink no-underline hover:bg-sunken focus-visible:outline-offset-[-2px]`}>
            {body}
          </a>
        ) : (
          <div key={i} className={cls}>
            {body}
          </div>
        );
      })}
    </div>
  );
}

export interface BarItem {
  label: ReactNode;
  value: number;
  /** Text on the right; defaults to value.toLocaleString(). */
  display?: string;
  href?: string;
}

/** Ranked list: label, bar scaled to the largest value, mono value. For "Disk by folder", "Top processes". */
export function BarList({
  items,
  labelWidth = 96,
  color = "blue",
  max,
}: {
  items: BarItem[];
  labelWidth?: number;
  color?: "blue" | "amber";
  max?: number;
}) {
  const top = max ?? Math.max(1, ...items.map((i) => i.value));
  return (
    <ul className="flex flex-col gap-3.5">
      {items.map((it, i) => (
        <li key={i} className="grid items-center gap-3" style={{ gridTemplateColumns: `${labelWidth}px minmax(0,1fr) 64px` }}>
          <div className="truncate text-sm text-ink">
            {it.href ? (
              <a href={it.href} className="text-ink no-underline hover:underline">
                {it.label}
              </a>
            ) : (
              it.label
            )}
          </div>
          <div className="h-2.5 rounded-full bg-muted" aria-hidden="true">
            <div
              className={`h-2.5 rounded-full ${color === "amber" ? "bg-chart-2" : "bg-chart-1"}`}
              style={{ width: `${Math.max(0, Math.min(100, (it.value / top) * 100))}%` }}
            />
          </div>
          <div className="text-right font-mono text-13 font-medium text-ink">{it.display ?? it.value.toLocaleString("en-US")}</div>
        </li>
      ))}
    </ul>
  );
}
