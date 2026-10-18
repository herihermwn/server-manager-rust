import type { ReactNode } from "react";

interface PageHeaderProps {
  title: ReactNode;
  /** One line of context: a count ("4 apps · 3 running"), a date, a status. */
  subtitle?: ReactNode;
  /** Right-aligned buttons. Keep to one primary. */
  actions?: ReactNode;
}

/** Page h1 (28/36, 22 on mobile) + subtitle + actions. The topbar breadcrumb already names the section, don't repeat it here. */
export function PageHeader({ title, subtitle, actions }: PageHeaderProps) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-3 lg:gap-4">
      <div className="flex min-w-0 flex-col gap-1">
        <h1 className="text-22 font-semibold text-ink lg:text-28">{title}</h1>
        {subtitle && <p className="text-sm text-ink-2">{subtitle}</p>}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}
