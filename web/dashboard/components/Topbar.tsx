import { useEffect, useState } from "react";
import { authFetch } from "../lib/api";
import { LiveDot, type Tone } from "./Badge";
import { BrandTile, ChevronRightIcon, MenuIcon } from "./icons";
import type { Crumb } from "./Sidebar";

/**
 * Round trip to the manager every 15 s. Topbar.dc.html shows "Server online · ping 38 ms",
 * or "Agent unreachable" when the API stops answering (tunnel closed, service down).
 */
// ponytail: only reachability. Phase 2 adds the "2 warnings · disk 92%" state from metrics here.
function useServerStatus(): { tone: Tone; text: string; meta: string } {
  const [ping, setPing] = useState<number | null | undefined>(undefined);
  const [since, setSince] = useState("");
  useEffect(() => {
    let live = true;
    const check = async () => {
      const t = performance.now();
      try {
        const r = await authFetch("/api/auth/me", { cache: "no-store" });
        if (live) setPing(r.ok ? Math.round(performance.now() - t) : null);
      } catch {
        if (!live) return;
        setPing(null);
        setSince((s) => s || new Date().toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" }));
      }
    };
    check();
    const id = setInterval(check, 15_000);
    return () => {
      live = false;
      clearInterval(id);
    };
  }, []);
  useEffect(() => {
    if (ping) setSince("");
  }, [ping]);
  if (ping === undefined) return { tone: "neutral", text: "Checking server", meta: "" };
  if (ping === null) return { tone: "danger", text: "Agent unreachable", meta: since ? `since ${since}` : "" };
  return { tone: "success", text: "Server online", meta: `ping ${ping} ms` };
}

/** Desktop bar (lg+): breadcrumb, server status pill. */
export function Topbar({ crumb }: { crumb: Crumb }) {
  const s = useServerStatus();
  return (
    <header className="hidden h-[72px] shrink-0 items-center justify-between gap-4 border-b border-line bg-paper px-8 lg:flex">
      <nav aria-label="Breadcrumb" className="min-w-0">
        <ol className="flex items-center gap-2 text-sm">
          <li className="shrink-0 text-ink-2">
            {crumb.sectionHref ? (
              <a href={crumb.sectionHref} className="text-ink-2 no-underline hover:text-ink hover:underline">
                {crumb.section}
              </a>
            ) : (
              crumb.section
            )}
          </li>
          <li aria-hidden="true" className="text-ink-3">
            <ChevronRightIcon size={14} />
          </li>
          <li aria-current="page" className="truncate font-semibold text-ink">
            {crumb.page}
          </li>
        </ol>
      </nav>
      <div className="flex shrink-0 items-center gap-3">
        <span role="status" className="inline-flex h-9 items-center gap-2 rounded-full border border-line bg-surface px-3 text-ink">
          <LiveDot tone={s.tone}>{s.text}</LiveDot>
          {s.meta && <span className="font-mono text-xs text-ink-3">{s.meta}</span>}
        </span>
      </div>
    </header>
  );
}

/** Mobile bar (below lg): menu button, brand, current page. */
export function MobileBar({ title, menuOpen, onMenu }: { title: string; menuOpen: boolean; onMenu: () => void }) {
  return (
    <header className="sticky top-0 z-30 flex h-16 shrink-0 items-center gap-1 bg-console pl-1 pr-2 text-white lg:hidden">
      <button
        type="button"
        onClick={onMenu}
        aria-label="Open menu"
        aria-expanded={menuOpen}
        className="inline-flex h-12 w-12 items-center justify-center rounded-lg hover:bg-console-raised"
      >
        <MenuIcon size={22} />
      </button>
      <BrandTile size={28} />
      <span className="min-w-0 flex-1 truncate pl-2 text-base font-semibold">{title}</span>
    </header>
  );
}
