import type { AnchorHTMLAttributes, ButtonHTMLAttributes, ReactNode } from "react";
import { Spinner } from "./icons";

/**
 * Variant follows what the action DOES, not how loud it should look:
 *   primary   – writes data (Deploy, Save, Create)
 *   secondary – neutral: Cancel, Back, Restart, Refresh
 *   subtle    – reads or opens: View log, Copy, Open site
 *   danger    – destroys (Delete app, Drop database)
 */
export type ButtonVariant = "primary" | "secondary" | "subtle" | "danger";
/** sm 36px (dense table rows only), md 44px (default), lg 48px (single form CTA like Sign in). */
export type ButtonSize = "sm" | "md" | "lg";

const VARIANT: Record<ButtonVariant, string> = {
  primary: "bg-ink text-ink-on hover:bg-ink-hover",
  secondary: "bg-surface text-ink border border-line-strong hover:bg-paper",
  subtle: "bg-info-bg text-info-strong hover:bg-info-hover",
  danger: "bg-danger text-ink-on hover:bg-danger-hover",
};

const SIZE: Record<ButtonSize, string> = {
  sm: "h-9 px-3 text-13 gap-1.5",
  md: "h-11 px-[18px] text-sm gap-2",
  lg: "h-12 px-5 text-15 gap-2.5",
};

const DISABLED = "bg-disabled-bg text-disabled cursor-not-allowed";

interface CommonProps {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Leading icon, e.g. <PlusIcon />. Replaced by a spinner while busy. */
  icon?: ReactNode;
  /** Shows a spinner, sets aria-busy and blocks clicks. Pass the -ing label as children ("Saving…"). */
  busy?: boolean;
  className?: string;
  children: ReactNode;
}

type AsButton = CommonProps & Omit<ButtonHTMLAttributes<HTMLButtonElement>, keyof CommonProps> & { href?: undefined };
/** With `href` the button renders an <a> (use for navigation: "#/apps", "#/apps/new"). */
type AsLink = CommonProps & Omit<AnchorHTMLAttributes<HTMLAnchorElement>, keyof CommonProps> & { href: string };
export type ButtonProps = AsButton | AsLink;

export function Button(props: ButtonProps) {
  const { variant = "primary", size = "md", icon, busy = false, className = "", children, ...rest } = props;
  const disabled = "disabled" in rest && !!rest.disabled && !busy;
  const cls = [
    "inline-flex shrink-0 select-none items-center justify-center whitespace-nowrap rounded-lg font-semibold no-underline transition-colors",
    SIZE[size],
    icon || busy ? (size === "md" ? "pl-[14px]" : "") : "",
    disabled ? DISABLED : VARIANT[variant],
    busy ? "cursor-progress" : "",
    className,
  ].join(" ");
  const content = (
    <>
      {busy ? <Spinner size={size === "sm" ? 14 : 16} /> : icon}
      {children}
    </>
  );

  if (rest.href !== undefined) {
    const a = rest as AnchorHTMLAttributes<HTMLAnchorElement>;
    return (
      <a {...a} className={cls} aria-busy={busy || undefined}>
        {content}
      </a>
    );
  }
  const b = rest as ButtonHTMLAttributes<HTMLButtonElement>;
  return (
    <button type="button" {...b} disabled={b.disabled || busy} aria-busy={busy || undefined} className={cls}>
      {content}
    </button>
  );
}

/**
 * Square icon-only button. `label` is required: it becomes aria-label and the tooltip.
 *   secondary – bordered (Refresh, row menu)
 *   ghost     – transparent (More actions, modal close, password eye)
 *   console   – on the dark sidebar / mobile bar
 */
export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  label: string;
  children: ReactNode;
  variant?: "secondary" | "ghost" | "console";
  size?: "sm" | "md";
  busy?: boolean;
}

const ICON_VARIANT = {
  secondary: "bg-surface text-ink border border-line-strong hover:bg-paper",
  ghost: "bg-transparent text-ink-2 hover:bg-muted hover:text-ink",
  console: "bg-transparent text-console-dim hover:bg-console-raised hover:text-white",
};

export function IconButton({
  label,
  children,
  variant = "secondary",
  size = "md",
  busy = false,
  className = "",
  disabled,
  ...rest
}: IconButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      {...rest}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={[
        "inline-flex shrink-0 items-center justify-center rounded-lg transition-colors",
        size === "md" ? "h-11 w-11" : "h-9 w-9",
        disabled ? "cursor-not-allowed text-disabled" : ICON_VARIANT[variant],
        className,
      ].join(" ")}
    >
      {busy ? <Spinner /> : children}
    </button>
  );
}
