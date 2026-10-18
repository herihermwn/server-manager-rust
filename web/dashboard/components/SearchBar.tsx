import { useId, type FormEvent } from "react";
import { Button } from "./Button";
import { CloseIcon, SearchIcon } from "./icons";

interface SearchBarProps {
  value: string;
  onChange: (value: string) => void;
  /** Enter or the Search button. */
  onSearch: () => void;
  /** Clear (x) button. Defaults to onChange(""); pass this to also re-run the search. */
  onClear?: () => void;
  placeholder?: string;
  /** Accessible name of the field (visually hidden). */
  label?: string;
  buttonText?: string;
  busy?: boolean;
  className?: string;
}

/** Search field + primary Search button in a role="search" form. */
export function SearchBar({
  value,
  onChange,
  onSearch,
  onClear,
  placeholder = "Search",
  label = "Search",
  buttonText = "Search",
  busy = false,
  className = "",
}: SearchBarProps) {
  const id = useId();
  const submit = (e: FormEvent) => {
    e.preventDefault();
    onSearch();
  };
  return (
    <form role="search" onSubmit={submit} className={`flex gap-2 ${className}`}>
      <label htmlFor={id} className="sr-only">
        {label}
      </label>
      <div className="flex h-11 min-w-0 flex-1 items-center gap-2 rounded-lg border border-line-strong bg-surface pl-3 pr-1 has-[input:focus-visible]:border-info has-[input:focus-visible]:outline has-[input:focus-visible]:outline-2 has-[input:focus-visible]:outline-offset-1 has-[input:focus-visible]:outline-info">
        <SearchIcon className="text-ink-3" />
        <input
          id={id}
          type="search"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          className="h-full min-w-0 flex-1 bg-transparent text-sm text-ink placeholder:text-ink-3 focus-visible:outline-none [&::-webkit-search-cancel-button]:hidden"
        />
        {value && (
          <button
            type="button"
            onClick={() => (onClear ? onClear() : onChange(""))}
            aria-label="Clear search"
            className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-md text-ink-3 hover:bg-muted hover:text-ink"
          >
            <CloseIcon size={16} strokeWidth={2.25} />
          </button>
        )}
      </div>
      <Button type="submit" busy={busy}>
        {buttonText}
      </Button>
    </form>
  );
}
