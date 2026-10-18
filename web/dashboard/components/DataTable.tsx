import type { MouseEvent, ReactNode } from "react";
import { EmptyState, Skeleton } from "./Feedback";

export { Pagination } from "./Pagination";

export interface Column<T> {
  key: string;
  header: ReactNode;
  /** Defaults to String(item[key]), or an em dash when empty. */
  render?: (item: T) => ReactNode;
  /** Numbers, dates-as-durations, counts: right-aligned. */
  align?: "left" | "right";
  /** Mono 13px (ids, counts, codes, durations). Combine with align="right" for numbers. */
  mono?: boolean;
  /** CSS width, e.g. 120 or "30%". */
  width?: number | string;
  /** Extra classes on both th and td. */
  className?: string;
}

interface DataTableProps<T> {
  columns: Column<T>[];
  data: T[];
  /** Renders `skeletonRows` placeholder rows instead of data. */
  loading?: boolean;
  skeletonRows?: number;
  /** Shorthand for the empty slot: an EmptyState with this title. */
  emptyMessage?: string;
  /** Custom empty slot, e.g. <EmptyState title=… action=… />. Wins over emptyMessage. */
  empty?: ReactNode;
  /** Row is a link: the first column's content becomes an <a href>, and the whole row is clickable. */
  rowHref?: (item: T) => string;
  onRowClick?: (item: T) => void;
  rowKey?: (item: T, index: number) => string | number;
  /** Accessible name of the table. */
  label?: string;
  /** false when the table sits inside a <Section flush> (which already draws the card). */
  framed?: boolean;
  /** Horizontal scroll kicks in below this width. */
  minWidth?: number;
}

const INTERACTIVE = "a,button,input,select,textarea,label,[role=button]";

export function DataTable<T extends { id?: number | string }>({
  columns,
  data,
  loading,
  skeletonRows = 5,
  emptyMessage = "Nothing here yet",
  empty,
  rowHref,
  onRowClick,
  rowKey,
  label,
  framed = true,
  minWidth = 640,
}: DataTableProps<T>) {
  const cell = (c: Column<T>) =>
    `px-3 first:pl-6 last:pr-6 ${c.align === "right" ? "text-right" : "text-left"} ${c.className ?? ""}`;

  const clickable = !!(rowHref || onRowClick);
  const onClick = (item: T) => (e: MouseEvent<HTMLTableRowElement>) => {
    if ((e.target as HTMLElement).closest(INTERACTIVE)) return; // let links/buttons inside the row work
    if (onRowClick) onRowClick(item);
    else if (rowHref) window.location.href = rowHref(item);
  };

  const value = (c: Column<T>, item: T, first: boolean) => {
    const raw = c.render ? c.render(item) : (item as Record<string, unknown>)[c.key];
    const v = raw == null || raw === "" ? <span className="text-ink-3">—</span> : c.render ? raw : String(raw);
    return first && rowHref ? (
      <a href={rowHref(item)} className="font-medium text-ink no-underline hover:underline">
        {v as ReactNode}
      </a>
    ) : (
      (v as ReactNode)
    );
  };

  return (
    <div className={framed ? "overflow-hidden rounded-xl border border-line bg-surface" : ""}>
      <div className="overflow-x-auto">
        <table className="w-full border-collapse text-sm" style={{ minWidth }} aria-label={label} aria-busy={loading || undefined}>
          <thead>
            <tr className="bg-sunken">
              {columns.map((c) => (
                <th
                  key={c.key}
                  scope="col"
                  style={c.width != null ? { width: c.width } : undefined}
                  className={`whitespace-nowrap border-b border-line py-2.5 text-xs font-semibold uppercase tracking-caps text-ink-3 ${cell(c)}`}
                >
                  {c.header}
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="[&>tr:last-child>td]:border-b-0">
            {loading ? (
              Array.from({ length: skeletonRows }, (_, r) => (
                <tr key={r}>
                  {columns.map((c, i) => (
                    <td key={c.key} className={`h-[52px] border-b border-line-soft ${cell(c)}`}>
                      <Skeleton className={`h-3 ${i === 0 ? "w-3/4" : "w-1/2"} ${c.align === "right" ? "ml-auto" : ""}`} />
                    </td>
                  ))}
                </tr>
              ))
            ) : data.length === 0 ? (
              <tr>
                <td colSpan={columns.length}>{empty ?? <EmptyState title={emptyMessage} />}</td>
              </tr>
            ) : (
              data.map((item, idx) => (
                <tr
                  key={rowKey ? rowKey(item, idx) : (item.id ?? idx)}
                  onClick={clickable ? onClick(item) : undefined}
                  className={`transition-colors hover:bg-sunken ${clickable ? "cursor-pointer" : ""}`}
                >
                  {columns.map((c, i) => (
                    <td
                      key={c.key}
                      className={`h-[52px] border-b border-line-soft ${c.mono ? "font-mono text-13" : ""} ${cell(c)}`}
                    >
                      {value(c, item, i === 0)}
                    </td>
                  ))}
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
