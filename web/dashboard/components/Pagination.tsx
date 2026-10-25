import { ChevronLeftIcon, ChevronRightIcon } from "./icons";

interface PaginationProps {
  /** 1-based. */
  page: number;
  totalPages: number;
  onPageChange: (page: number) => void;
  /** With total + pageSize the left side reads "21–40 of 106,797"; otherwise "Page 2 of 5,340". */
  total?: number;
  pageSize?: number;
  className?: string;
}

const fmt = (n: number) => n.toLocaleString("en-US");

/** 1 … 4 5 6 … 20 */
export function pageList(page: number, totalPages: number): (number | "gap")[] {
  const want = [...new Set([1, totalPages, page - 1, page, page + 1])].filter((p) => p >= 1 && p <= totalPages).sort((a, b) => a - b);
  const out: (number | "gap")[] = [];
  for (const p of want) {
    const prev = out[out.length - 1];
    if (typeof prev === "number" && p - prev === 2) out.push(prev + 1);
    else if (typeof prev === "number" && p - prev > 2) out.push("gap");
    out.push(p);
  }
  return out;
}

const btn = "inline-flex h-10 min-w-10 items-center justify-center rounded-lg px-2 font-mono text-13 font-medium";
const idle = `${btn} border border-line bg-surface text-ink hover:bg-paper disabled:cursor-not-allowed disabled:text-disabled disabled:hover:bg-surface`;

export function Pagination({ page, totalPages, onPageChange, total, pageSize, className = "" }: PaginationProps) {
  if (totalPages <= 1 && total == null) return null;
  const range =
    total != null && pageSize ? (
      <>
        <span className="font-mono text-ink">
          {total === 0 ? 0 : fmt((page - 1) * pageSize + 1)}–{fmt(Math.min(page * pageSize, total))}
        </span>{" "}
        of <span className="font-mono text-ink">{fmt(total)}</span>
      </>
    ) : (
      <>
        Page <span className="font-mono text-ink">{fmt(page)}</span> of <span className="font-mono text-ink">{fmt(Math.max(totalPages, 1))}</span>
      </>
    );

  return (
    <nav aria-label="Pagination" className={`flex flex-wrap items-center justify-between gap-3 ${className}`}>
      <div className="text-13 text-ink-2">{range}</div>
      {totalPages > 1 && (
        <div className="flex gap-1">
          <button type="button" className={idle} aria-label="Previous page" disabled={page <= 1} onClick={() => onPageChange(page - 1)}>
            <ChevronLeftIcon size={16} />
          </button>
          {pageList(page, totalPages).map((p, i) =>
            p === "gap" ? (
              <span key={`gap-${i}`} className={`${btn} hidden text-ink-3 sm:inline-flex`} aria-hidden="true">
                …
              </span>
            ) : p === page ? (
              <button key={p} type="button" className={`${btn} bg-ink text-ink-on`} aria-current="page" aria-label={`Page ${p}, current`}>
                {fmt(p)}
              </button>
            ) : (
              <button
                key={p}
                type="button"
                className={`${idle} ${Math.abs(p - page) > 1 ? "hidden sm:inline-flex" : ""}`}
                aria-label={`Page ${p}`}
                onClick={() => onPageChange(p)}
              >
                {fmt(p)}
              </button>
            )
          )}
          <button type="button" className={idle} aria-label="Next page" disabled={page >= totalPages} onClick={() => onPageChange(page + 1)}>
            <ChevronRightIcon size={16} />
          </button>
        </div>
      )}
    </nav>
  );
}
