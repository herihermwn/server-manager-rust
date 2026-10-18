import { useEffect, useState } from "react";
import { authFetch } from "../lib/api";
import { LiveDot, type Tone } from "./Badge";
import { BrandTile, ChevronRightIcon, MenuIcon } from "./icons";
import type { Crumb } from "./Sidebar";

/**
 * Round trip to the manager every 15 s (Topbar.dc.html): "Server online · ping 38 ms", "N warnings ·
 * disk 92%" when /api/metrics/alerts has any, "Agent unreachable" when the API stops answering
 * (tunnel closed, service down).
 */
function useServerStatus(): { tone: Tone; text: string; meta: string } {
  const [ping, setPing] = useState<number | null | undefined>(undefined);
  const [alerts, setAlerts] = useState<{ short: string }[]>([]);
  const [since, setSince] = useState("");
  useEffect(() => {
    let live = true;
    const check = async () => {
      const t = performance.now();
      try {
        const r = await authFetch("/api/auth/me", { cache: "no-store" });
        if (live) setPing(r.ok ? Math.round(performance.now() - t) : null);
        if (!r.ok) return;
        const a = await authFetch("/api/metrics/alerts", { cache: "no-store" });
        if (live && a.ok) setAlerts(((await a.json()) as { alerts?: { short: string }[] }).alerts ?? []);
      } catch {
        if (!live) return;
        setPing(null);
        setSince((s) => s || new Date().toLocaleTimeString("en-GB", { timeZone: "Asia/Jakarta", hour: "2-digit", minute: "2-digit" }));
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
  if (alerts.length > 0) return { tone: "warning", text: alerts.length === 1 ? "1 warning" : `${alerts.length} warnings`, meta: alerts[0]?.short ?? "" };
  return { tone: "success", text: "Server online", meta: `ping ${ping} ms` };
}

interface ActiveDeploy {
  app: string;
  deployId: number;
  startedAt: number;
}

/** GET /api/deploys/active (phase 3): { data: [{ app, deployId, startedAt }] }. 404 = not built yet, ignored. */
function useActiveDeploys(): ActiveDeploy[] {
  const [list, setList] = useState<ActiveDeploy[]>([]);
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      let next = 5_000;
      try {
        const r = await authFetch("/api/deploys/active", { cache: "no-store" });
        if (r.ok) {
          const body = (await r.json()) as { data?: ActiveDeploy[] };
          if (live) setList(body.data ?? []);
        } else {
          if (live) setList([]);
          if (r.status === 404) next = 60_000;
        }
      } catch {
        if (live) setList([]);
      }
      if (live) timer = setTimeout(poll, next);
    };
    poll();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, []);
  return list;
}

/** "Deploying radio-api-rs 0:48": ticks every second, links to the app. */
function DeployPill({ deploys }: { deploys: ActiveDeploy[] }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  const first = deploys[0];
  if (!first) return null;
  const started = first.startedAt > 1e11 ? first.startedAt : first.startedAt * 1000;
  const secs = Math.max(0, Math.floor((now - started) / 1000));
  const many = deploys.length > 1;
  return (
    <a
      href={many ? "#/apps" : `#/apps/${first.app}`}
      className="inline-flex h-9 items-center gap-2 rounded-full bg-info-bg px-3 text-13 font-semibold text-info-ink no-underline hover:bg-info-hover"
    >
      <LiveDot tone="info">{many ? `Deploying ${deploys.length} apps` : `Deploying ${first.app}`}</LiveDot>
      {!many && (
        <span className="font-mono text-xs font-medium">
          {Math.floor(secs / 60)}:{String(secs % 60).padStart(2, "0")}
        </span>
      )}
    </a>
  );
}

/** Desktop bar (lg+): breadcrumb, server status pill. */
export function Topbar({ crumb }: { crumb: Crumb }) {
  const s = useServerStatus();
  const deploys = useActiveDeploys();
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
        {deploys.length > 0 && <DeployPill deploys={deploys} />}
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
