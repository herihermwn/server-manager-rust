import { useEffect, useId, useRef, useState, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { Button, IconButton } from "./Button";
import { AlertTriangleIcon, CloseIcon } from "./icons";

const FOCUSABLE = 'a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])';

/**
 * Shared overlay behaviour: Esc closes, Tab stays inside, body scroll locked,
 * focus moves in on open (initialFocus, else first field/button) and returns to the opener on close.
 * ponytail: basic focus trap, no inert on siblings; swap for <dialog> if nested overlays appear.
 */
export function useDialog(open: boolean, onClose: () => void, panel: RefObject<HTMLElement | null>, initialFocus?: RefObject<HTMLElement | null>) {
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    if (!open) return;
    const opener = document.activeElement as HTMLElement | null;
    const el = panel.current;
    const items = () => Array.from(el?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? []);
    const first = initialFocus?.current ?? el?.querySelector<HTMLElement>("input,select,textarea") ?? items()[0] ?? el;
    first?.focus();

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        closeRef.current();
      } else if (e.key === "Tab") {
        const list = items();
        if (list.length === 0) return e.preventDefault();
        const a = list[0]!;
        const z = list[list.length - 1]!;
        if (e.shiftKey && document.activeElement === a) {
          e.preventDefault();
          z.focus();
        } else if (!e.shiftKey && document.activeElement === z) {
          e.preventDefault();
          a.focus();
        }
      }
    };
    document.addEventListener("keydown", onKey);
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = overflow;
      opener?.focus?.();
    };
  }, [open]);
}

const SIZE = { sm: "max-w-[440px]", md: "max-w-[520px]", lg: "max-w-2xl", xl: "max-w-4xl" };

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  /** One line under the title saying what happens next. */
  description?: ReactNode;
  children?: ReactNode;
  /** Buttons, right-aligned on a sunken strip. Order: Cancel (secondary) … primary action last. */
  footer?: ReactNode;
  size?: keyof typeof SIZE;
  initialFocus?: RefObject<HTMLElement | null>;
}

/** Centered dialog (forms: Create database, Edit site). For "are you sure" use ConfirmDialog. */
export function Modal({ open, onClose, title, description, children, footer, size = "md", initialFocus }: ModalProps) {
  const panel = useRef<HTMLDivElement>(null);
  const id = useId();
  useDialog(open, onClose, panel, initialFocus);
  if (!open) return null;

  return createPortal(
    <div className="fixed inset-0 z-50 overflow-y-auto">
      <div className="fixed inset-0 bg-ink/[0.48]" onClick={onClose} aria-hidden="true" />
      <div className="flex min-h-full items-center justify-center p-4">
        <div
          ref={panel}
          role="dialog"
          aria-modal="true"
          aria-labelledby={`${id}-t`}
          aria-describedby={description ? `${id}-d` : undefined}
          tabIndex={-1}
          className={`relative w-full overflow-hidden rounded-2xl bg-surface shadow-modal focus-visible:outline-none ${SIZE[size]}`}
        >
          <div className="flex items-start justify-between gap-4 pl-6 pr-5 pt-5">
            <div className="flex flex-col gap-1 pt-1">
              <h2 id={`${id}-t`} className="text-lg font-semibold leading-[26px] text-ink">
                {title}
              </h2>
              {description && (
                <div id={`${id}-d`} className="text-sm text-ink-2">
                  {description}
                </div>
              )}
            </div>
            <IconButton label="Close" variant="ghost" onClick={onClose}>
              <CloseIcon strokeWidth={2.25} />
            </IconButton>
          </div>
          {children != null && <div className="px-6 py-5">{children}</div>}
          {footer && <div className="flex flex-wrap justify-end gap-2 border-t border-line bg-sunken px-6 py-4">{footer}</div>}
        </div>
      </div>
    </div>,
    document.body
  );
}

interface ConfirmDialogProps {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  /** A question naming the action: "Delete radio-api-rs?" */
  title: ReactNode;
  /** Name the effect and the records: "<strong>radio-api-rs</strong> stops, its systemd unit and folder are removed. Databases stay." */
  children: ReactNode;
  /** Verb + object, never "OK": "Delete app", "Drop 2 databases". */
  confirmLabel: string;
  /** The safe choice, phrased as staying: "Keep pending". */
  cancelLabel?: string;
  busy?: boolean;
  /** danger (default) for destroy / dismiss, primary for a non-destructive but weighty write. */
  tone?: "danger" | "primary";
  /** Type-to-confirm (Overlays.dc.html): the confirm button stays disabled until this exact text is typed. */
  confirmText?: string;
}

/** role="alertdialog". Focus starts on Cancel so Enter never destroys by accident. */
export function ConfirmDialog({
  open,
  onClose,
  onConfirm,
  title,
  children,
  confirmLabel,
  cancelLabel = "Cancel",
  busy,
  tone = "danger",
  confirmText,
}: ConfirmDialogProps) {
  const panel = useRef<HTMLDivElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const id = useId();
  const [typed, setTyped] = useState("");
  useEffect(() => setTyped(""), [open]);
  useDialog(open, busy ? () => {} : onClose, panel, cancel);
  if (!open) return null;
  const blocked = confirmText !== undefined && typed !== confirmText;

  return createPortal(
    <div className="fixed inset-0 z-50 overflow-y-auto">
      <div className="fixed inset-0 bg-ink/[0.48]" onClick={busy ? undefined : onClose} aria-hidden="true" />
      <div className="flex min-h-full items-center justify-center p-4">
        <div
          ref={panel}
          role="alertdialog"
          aria-modal="true"
          aria-labelledby={`${id}-t`}
          aria-describedby={`${id}-d`}
          className="relative flex w-full max-w-[440px] flex-col gap-4 rounded-2xl bg-surface p-6 shadow-modal"
        >
          {tone === "danger" && (
            <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-danger-bg text-danger">
              <AlertTriangleIcon size={20} strokeWidth={2.25} />
            </div>
          )}
          <div className="flex flex-col gap-1.5">
            <h2 id={`${id}-t`} className="text-lg font-semibold leading-[26px] text-ink">
              {title}
            </h2>
            <div id={`${id}-d`} className="text-sm leading-[21px] text-ink-2 [&_strong]:font-semibold [&_strong]:text-ink">
              {children}
            </div>
          </div>
          {confirmText !== undefined && (
            <div className="flex flex-col gap-1.5">
              <label htmlFor={`${id}-c`} className="text-13 font-semibold text-ink">
                Type <span className="font-mono">{confirmText}</span> to confirm
              </label>
              <input
                id={`${id}-c`}
                value={typed}
                onChange={(e) => setTyped(e.target.value)}
                autoComplete="off"
                spellCheck={false}
                className="h-11 rounded-lg border border-line-strong bg-surface px-3 font-mono text-sm text-ink focus-visible:outline-offset-1"
              />
            </div>
          )}
          <div className="flex flex-wrap justify-end gap-2">
            <button
              ref={cancel}
              type="button"
              onClick={onClose}
              disabled={busy}
              className="inline-flex h-11 items-center rounded-lg border border-line-strong bg-surface px-[18px] text-sm font-semibold text-ink hover:bg-paper disabled:cursor-not-allowed disabled:text-disabled"
            >
              {cancelLabel}
            </button>
            <Button variant={tone} onClick={onConfirm} busy={busy} disabled={blocked}>
              {confirmLabel}
            </Button>
          </div>
        </div>
      </div>
    </div>,
    document.body
  );
}

interface DrawerProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  /** px, capped at the viewport. */
  width?: number;
}

/** Right-side panel for detail / edit without leaving the list (e.g. deploy details, filters on mobile). */
export function Drawer({ open, onClose, title, description, children, footer, width = 480 }: DrawerProps) {
  const panel = useRef<HTMLDivElement>(null);
  const id = useId();
  useDialog(open, onClose, panel);
  if (!open) return null;

  return createPortal(
    <div className="fixed inset-0 z-50">
      <div className="absolute inset-0 bg-ink/[0.48]" onClick={onClose} aria-hidden="true" />
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${id}-t`}
        tabIndex={-1}
        style={{ width }}
        className="absolute inset-y-0 right-0 flex max-w-full flex-col bg-surface shadow-modal focus-visible:outline-none"
      >
        <div className="flex items-start justify-between gap-4 border-b border-line py-4 pl-6 pr-4">
          <div className="flex min-w-0 flex-col gap-1 pt-1.5">
            <h2 id={`${id}-t`} className="text-lg font-semibold leading-[26px] text-ink">
              {title}
            </h2>
            {description && <div className="text-sm text-ink-2">{description}</div>}
          </div>
          <IconButton label="Close" variant="ghost" onClick={onClose}>
            <CloseIcon strokeWidth={2.25} />
          </IconButton>
        </div>
        <div className="flex-1 overflow-y-auto px-6 py-5">{children}</div>
        {footer && <div className="flex flex-wrap justify-end gap-2 border-t border-line bg-sunken px-6 py-4">{footer}</div>}
      </div>
    </div>,
    document.body
  );
}
