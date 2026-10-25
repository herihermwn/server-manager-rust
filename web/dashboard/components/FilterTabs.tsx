export interface FilterOption<T extends string = string> {
  value: T;
  label: string;
  /** Mono count after the label ("5,093"). Numbers are formatted with toLocaleString. */
  count?: number | string;
}

interface FilterTabsProps<T extends string = string> {
  options: FilterOption<T>[];
  value: T;
  onChange: (value: T) => void;
  /** Accessible name for the group, e.g. "App status". */
  label?: string;
}

/** Status filter on a gray track; the selected tab is a white raised pill. Use above a list. */
export function FilterTabs<T extends string = string>({ options, value, onChange, label = "Filter" }: FilterTabsProps<T>) {
  return (
    <div role="group" aria-label={label} className="inline-flex max-w-full gap-1 self-start overflow-x-auto rounded-[10px] bg-muted p-1">
      {options.map((o) => {
        const sel = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={sel}
            onClick={() => onChange(o.value)}
            className={`inline-flex h-9 shrink-0 items-center gap-2 whitespace-nowrap rounded-[7px] px-3 text-13 text-ink ${
              sel ? "bg-surface font-semibold shadow-tab" : "font-medium hover:bg-surface/60"
            }`}
          >
            {o.label}
            {o.count != null && (
              <span className={`font-mono text-xs font-medium ${sel ? "text-ink-2" : "text-ink-3"}`}>
                {typeof o.count === "number" ? o.count.toLocaleString("en-US") : o.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

interface SegmentedControlProps<T extends string = string> {
  options: { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
  label: string;
}

/** Bordered segmented switch for a view setting (24 h / 7 days / 30 days). Selected segment is ink. */
export function SegmentedControl<T extends string = string>({ options, value, onChange, label }: SegmentedControlProps<T>) {
  return (
    <div role="group" aria-label={label} className="inline-flex shrink-0 overflow-hidden rounded-lg border border-line-strong bg-surface">
      {options.map((o, i) => {
        const sel = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={sel}
            onClick={() => onChange(o.value)}
            className={`h-[42px] whitespace-nowrap px-4 text-13 focus-visible:outline-offset-[-2px] ${i > 0 ? "border-l border-line-strong" : ""} ${
              sel ? "bg-ink font-semibold text-ink-on" : "font-medium text-ink hover:bg-paper"
            }`}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
