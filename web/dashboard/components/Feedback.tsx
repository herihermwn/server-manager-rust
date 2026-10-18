import type { ReactNode } from "react";
import { Button } from "./Button";
import { AlertCircleIcon, AlertTriangleIcon, CheckIcon, EmptySearchIcon } from "./icons";

/** Gray placeholder block. Size it with className: <Skeleton className="h-7 w-2/3" />. Same shape as the content. */
export function Skeleton({ className = "h-3 w-full", light }: { className?: string; light?: boolean }) {
  return <div aria-hidden="true" className={`animate-pulse rounded ${light ? "bg-paper" : "bg-muted"} ${className}`} />;
}

/** Card-sized loading placeholder: label, value, three text lines. */
export function SkeletonBlock({ label = "Loading" }: { label?: string }) {
  return (
    <div role="status" aria-busy="true" aria-label={label} className="flex flex-col gap-3.5">
      <Skeleton className="h-3.5 w-2/5" />
      <Skeleton className="h-7 w-[70%] rounded-md" />
      <Skeleton light className="h-3 w-[90%]" />
      <Skeleton light className="h-3 w-[82%]" />
      <Skeleton light className="h-3 w-3/5" />
    </div>
  );
}

/**
 * Nothing to show. Say what is empty and how to get out of it:
 * title "No apps yet", description "Add one from a git link. The first deploy runs right after the clone.",
 * action <Button href="#/apps/new">Add app</Button>.
 * Unframed: put it inside a Card / Section / DataTable `empty` slot.
 */
export function EmptyState({
  title,
  description,
  action,
  icon,
}: {
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  icon?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center gap-2.5 px-6 py-7 text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-muted text-ink-2">{icon ?? <EmptySearchIcon size={22} />}</div>
      <div className="text-15 font-semibold text-ink">{title}</div>
      {description && <div className="max-w-md text-13 leading-5 text-ink-2">{description}</div>}
      {action && <div className="mt-1.5">{action}</div>}
    </div>
  );
}

/** Inline load failure + retry. Say what failed and that nothing was lost ("Your filters are kept."). */
export function ErrorState({
  title,
  description,
  onRetry,
  retryLabel = "Try again",
  busy,
}: {
  title: ReactNode;
  description?: ReactNode;
  onRetry?: () => void;
  retryLabel?: string;
  busy?: boolean;
}) {
  return (
    <div className="flex flex-col items-start gap-3.5">
      <InlineAlert title={title}>{description}</InlineAlert>
      {onRetry && (
        <Button variant="secondary" onClick={onRetry} busy={busy}>
          {busy ? "Retrying…" : retryLabel}
        </Button>
      )}
    </div>
  );
}

/** Red alert box (form errors, failed loads). role="alert". */
export function InlineAlert({ title, children }: { title: ReactNode; children?: ReactNode }) {
  return (
    <div role="alert" className="flex w-full gap-3 rounded-[10px] bg-danger-bg px-4 py-3.5 text-danger-ink">
      <AlertCircleIcon size={20} strokeWidth={2.25} className="mt-px text-danger" />
      <div className="flex flex-col gap-1">
        <div className="text-sm font-semibold">{title}</div>
        {children && <div className="text-13 leading-[19px]">{children}</div>}
      </div>
    </div>
  );
}

type BannerTone = "info" | "warning" | "error" | "success";

const BANNER: Record<BannerTone, { box: string; action: string }> = {
  info: { box: "bg-info-bg text-info-ink", action: "border-info-ring text-info-ink" },
  warning: { box: "bg-warning-bg text-warning-ink", action: "border-warning-line text-warning-ink" },
  error: { box: "bg-danger-bg text-danger-ink", action: "border-danger/30 text-danger-ink" },
  success: { box: "bg-success-bg text-success", action: "border-success/30 text-success" },
};

function BannerIcon({ tone }: { tone: BannerTone }) {
  if (tone === "warning") return <AlertTriangleIcon size={20} strokeWidth={2.25} className="text-warning" />;
  if (tone === "error") return <AlertCircleIcon size={20} strokeWidth={2.25} className="text-danger" />;
  if (tone === "success") return <CheckIcon size={20} strokeWidth={2.25} className="text-success" />;
  return <span className="mx-1.5 h-2 w-2 shrink-0 rounded-full bg-info" style={{ boxShadow: "0 0 0 4px #CFE0F8" }} aria-hidden="true" />;
}

/**
 * Page-level message above the content. info = something is running ("Deploying radio-api-rs #43."),
 * warning = something needs attention ("Disk 92% full."), error = page-wide failure.
 * `title` is bold and runs into `children`.
 */
export function Banner({
  tone = "info",
  title,
  children,
  action,
}: {
  tone?: BannerTone;
  title?: ReactNode;
  children?: ReactNode;
  action?: { label: string; onClick?: () => void; href?: string };
}) {
  const t = BANNER[tone];
  const actionCls = `inline-flex h-11 shrink-0 items-center rounded-lg border bg-surface px-4 text-sm font-semibold no-underline hover:bg-paper ${t.action}`;
  return (
    <div
      role={tone === "error" || tone === "warning" ? "alert" : "status"}
      className={`flex items-center gap-3 rounded-xl py-3 pl-4 ${action ? "pr-3" : "pr-4"} ${t.box}`}
    >
      <BannerIcon tone={tone} />
      <div className="flex-1 text-sm leading-5">
        {title && <strong className="font-semibold">{title}</strong>} {children}
      </div>
      {action &&
        (action.href ? (
          <a href={action.href} className={actionCls}>
            {action.label}
          </a>
        ) : (
          <button type="button" onClick={action.onClick} className={actionCls}>
            {action.label}
          </button>
        ))}
    </div>
  );
}
