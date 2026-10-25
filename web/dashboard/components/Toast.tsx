import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { CloseIcon } from "./icons";

export interface ToastInput {
  title: string;
  body?: string;
  /** Dot color: success (green), info (blue), error (red). */
  tone?: "success" | "info" | "error";
  /** One follow-up: { label: "View log", href: "#/apps/radio-api-rs" } or { label: "Retry", onClick }. */
  action?: { label: string; href?: string; onClick?: () => void };
}

interface ToastItem extends ToastInput {
  id: number;
}

const ToastContext = createContext<((t: ToastInput) => void) | null>(null);

/**
 * const toast = useToast(); toast({ title: "Environment saved", body: "3 keys changed", tone: "success" })
 * Confirms something that already happened. Never the only place an error that needs action is shown.
 */
export function useToast() {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used inside <ToastProvider>");
  return ctx;
}

const DOT = { success: "#5BC48A", info: "#6FA2EE", error: "#F28B82" };
const HIDE_MS = 5000;

/** Bottom-right stack of dark toasts, each auto-hides after 5 s (paused while hovered or focused). */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const next = useRef(1);
  const dismiss = useCallback((id: number) => setItems((xs) => xs.filter((x) => x.id !== id)), []);
  const push = useCallback((t: ToastInput) => {
    const id = next.current++;
    setItems((xs) => [...xs.slice(-2), { ...t, id }]); // ponytail: keep at most 3 on screen
  }, []);

  return (
    <ToastContext.Provider value={push}>
      {children}
      <div aria-live="polite" className="pointer-events-none fixed bottom-4 right-4 z-[60] flex w-[min(400px,calc(100vw-32px))] flex-col gap-2">
        {items.map((t) => (
          <Toast key={t.id} item={t} onDismiss={() => dismiss(t.id)} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}

function Toast({ item, onDismiss }: { item: ToastItem; onDismiss: () => void }) {
  const [paused, setPaused] = useState(false);
  useEffect(() => {
    if (paused) return;
    const t = setTimeout(onDismiss, HIDE_MS);
    return () => clearTimeout(t);
  }, [paused]);

  const actionCls = "h-9 shrink-0 rounded-md px-3 text-13 font-semibold text-[#9DC0F5] no-underline hover:bg-white/10";
  return (
    <div
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
      onFocus={() => setPaused(true)}
      onBlur={() => setPaused(false)}
      className="pointer-events-auto flex items-start gap-3 rounded-xl bg-console py-3.5 pl-4 pr-2 text-console-text shadow-toast"
    >
      <span className="mt-[7px] h-2 w-2 shrink-0 rounded-full" style={{ background: DOT[item.tone ?? "success"] }} aria-hidden="true" />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="text-sm font-semibold">{item.title}</div>
        {item.body && <div className="text-13 text-console-dim">{item.body}</div>}
      </div>
      {item.action &&
        (item.action.href ? (
          <a href={item.action.href} onClick={onDismiss} className={`${actionCls} inline-flex items-center`}>
            {item.action.label}
          </a>
        ) : (
          <button
            type="button"
            onClick={() => {
              item.action?.onClick?.();
              onDismiss();
            }}
            className={actionCls}
          >
            {item.action.label}
          </button>
        ))}
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss notification"
        className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-md text-console-muted hover:bg-white/10 hover:text-white"
      >
        <CloseIcon size={16} strokeWidth={2.25} />
      </button>
    </div>
  );
}
