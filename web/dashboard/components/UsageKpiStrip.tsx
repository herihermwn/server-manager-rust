import type { ReactNode } from "react";
import { Skeleton } from "./Feedback";
import type { Level } from "../pages/usage/format";

export interface UsageKpi {
  label: string;
  value: ReactNode;
  unit?: ReactNode;
  meta?: ReactNode;
  /** ok = blue, warn = amber, bad = red (value and meter; meta text for warn/bad). */
  level?: Level;
  /** 0-100: draws a meter under the value (Overview). */
  pct?: number;
  /** Makes the cell a link (Overview cells open Usage). */
  href?: string;
}

const METER: Record<Level, string> = { ok: "bg-chart-1", warn: "bg-chart-2", bad: "bg-danger" };
const VALUE: Record<Level, string> = { ok: "text-ink", warn: "text-ink", bad: "text-danger" };
const META: Record<Level, string> = { ok: "text-ink-2", warn: "text-warning", bad: "text-danger" };

// Literal class strings so Tailwind keeps them. 5 cells: 1 / 2 / 5 per row; 6 cells: 2 / 3 / 6 per row.
const GRID: Record<number, string> = {
  5: "grid-cols-1 sm:grid-cols-2 xl:grid-cols-5 sm:[&>*:last-child]:col-span-2 xl:[&>*:last-child]:col-span-1",
  6: "grid-cols-2 lg:grid-cols-3 min-[1360px]:grid-cols-6",
};

/** Overview.dc.html / Usage.dc.html KPI strip: one card, cells split by 1px rules. */
export function UsageKpiStrip({ items, loading, label }: { items: UsageKpi[]; loading?: boolean; label: string }) {
  return (
    <section aria-label={label} className={`grid gap-px overflow-hidden rounded-xl border border-line bg-line ${GRID[items.length] ?? "grid-cols-2 lg:grid-cols-4"}`}>
      {items.map((k) => {
        const level = k.level ?? "ok";
        const body = (
          <>
            <div className="text-xs font-semibold uppercase tracking-caps text-ink-3">{k.label}</div>
            {loading ? (
              <>
                <Skeleton className="h-8 w-3/5 rounded-lg" />
                {k.pct !== undefined && <Skeleton className="h-1.5 w-full rounded-full" />}
                <Skeleton className="h-3 w-3/4" />
              </>
            ) : (
              <>
                <div className="flex items-baseline gap-1.5">
                  <span className={`text-28 font-semibold tabular-nums ${VALUE[level]}`}>{k.value}</span>
                  {k.unit && <span className="text-13 text-ink-2">{k.unit}</span>}
                </div>
                {k.pct !== undefined && (
                  <div
                    role="meter"
                    aria-label={k.label}
                    aria-valuenow={Math.round(k.pct)}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    className="h-1.5 overflow-hidden rounded-full bg-muted"
                  >
                    <div className={`h-full rounded-full ${METER[level]}`} style={{ width: `${Math.max(0, Math.min(100, k.pct))}%` }} />
                  </div>
                )}
                {k.meta != null && <div className={`text-13 ${META[level]}`}>{k.meta}</div>}
              </>
            )}
          </>
        );
        const cls = "flex min-w-0 flex-col gap-2 bg-surface p-5";
        return k.href ? (
          <a key={k.label} href={k.href} className={`${cls} text-ink no-underline hover:bg-sunken focus-visible:outline-offset-[-2px]`}>
            {body}
          </a>
        ) : (
          <div key={k.label} className={cls}>
            {body}
          </div>
        );
      })}
    </section>
  );
}
