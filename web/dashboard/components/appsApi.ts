import { useEffect, useState } from "react";
import { api } from "../lib/api";

/** Phase 3 API shapes (src/routes/apps.rs, src/routes/deploys.rs). Times are unix seconds. */

export type AppStatus = "running" | "stopped" | "crashed" | "deploying";
export type DeployStatus = "queued" | "running" | "success" | "failed" | "cancelled" | "rolled_back";

export interface LastDeploy {
  id: number;
  number: number;
  status: DeployStatus;
  finishedAt: number | null;
}

export interface AppSummary {
  id?: never;
  name: string;
  runtime: string;
  status: AppStatus;
  port: number | null;
  domain: string | null;
  branch: string;
  repo: string;
  commit: string | null;
  release: number | null;
  lastDeploy: LastDeploy | null;
  cpuPct: number | null;
  memBytes: number | null;
}

export interface Proc {
  activeState: string;
  subState: string;
  result: string;
  pid: number | null;
  restarts: number;
  startedAt: number | null;
  memBytes: number | null;
  cpuPct: number | null;
}

export interface Deploy {
  id: number;
  number: number;
  release: number | null;
  commit: string | null;
  message: string | null;
  branch: string;
  trigger: string;
  status: DeployStatus;
  error: string | null;
  actor: string | null;
  createdAt: number;
  startedAt: number | null;
  finishedAt: number | null;
}

export interface DeployStep {
  idx: number;
  name: string;
  command: string;
  status: "waiting" | "running" | "success" | "failed" | "skipped" | "cancelled";
  exitCode: number | null;
  startedAt: number | null;
  finishedAt: number | null;
}

export interface DeployDetail extends Deploy {
  steps: DeployStep[];
  cancellable: boolean;
}

export interface AppDetailData extends AppSummary {
  path: string;
  runAs: string;
  buildCmd: string;
  beforeRestartCmd: string;
  startCmd: string;
  healthPath: string;
  sharedPaths: string[];
  keepReleases: number;
  buildTimeoutSecs: number;
  autoDeploy: boolean;
  webhookPath: string;
  unit: string;
  process: Proc | null;
  live: Deploy | null;
  activeDeploy: { id: number; number: number } | null;
  deployCount: number;
  createdAt: number;
}

export interface DeployKey {
  publicKey: string;
  fingerprint: string;
  type: string;
}

export const appUrl = (name: string) => `/api/apps/${encodeURIComponent(name)}`;

const RUNTIME: Record<string, string> = { rust: "Rust", bun: "Bun", node: "Node.js", static: "Static", other: "Custom" };
export const runtimeLabel = (rt: string) => `${RUNTIME[rt] ?? "Custom"} · systemd`;

export function fmtBytes(b: number | null | undefined) {
  if (b == null) return "—";
  if (b < 1024 * 1024) return `${Math.round(b / 1024)} KB`;
  if (b < 1024 ** 3) return `${(b / 1024 / 1024).toFixed(b < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  return `${(b / 1024 ** 3).toFixed(1)} GB`;
}

/** 75 → "1m 15s", 3700 → "1h 01m". */
export function fmtDuration(secs: number | null | undefined) {
  if (secs == null || secs < 0) return "—";
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${String(secs % 60).padStart(2, "0")}s`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ${String(Math.floor((secs % 3600) / 60)).padStart(2, "0")}m`;
  return `${Math.floor(secs / 86400)}d ${Math.floor((secs % 86400) / 3600)}h`;
}

export const took = (d: { startedAt: number | null; finishedAt: number | null }, now = Date.now() / 1000) =>
  d.startedAt ? fmtDuration(Math.round((d.finishedAt ?? now) - d.startedAt)) : "—";

/** Webhook URL GitHub should call: `publicUrl` from useAuth() (SM_PUBLIC_URL on the server) + /hooks/<app>. */
export const webhookUrl = (path: string, publicUrl: string) => `${publicUrl}${path}`;

export const copy = (text: string) => navigator.clipboard?.writeText(text);

/**
 * GET `url` now and every `everyMs` (paused while the tab is hidden). `url = null` stays idle.
 * Keeps the last good data when a refresh fails.
 */
export function useApi<T>(url: string | null, everyMs = 0) {
  const [data, setData] = useState<T | undefined>();
  const [error, setError] = useState<Error | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!url) return;
    const ac = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = async () => {
      if (!document.hidden) {
        try {
          setData(await api<T>(url, { signal: ac.signal }));
          setError(null);
        } catch (e) {
          if (!ac.signal.aborted) setError(e as Error);
        }
      }
      if (everyMs > 0 && !ac.signal.aborted) timer = setTimeout(run, everyMs);
    };
    run();
    return () => {
      ac.abort();
      clearTimeout(timer);
    };
  }, [url, everyMs, tick]);
  return { data, error, loading: data === undefined && error === null, reload: () => setTick((t) => t + 1) };
}
