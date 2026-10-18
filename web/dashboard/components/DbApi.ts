import { formatWib } from "../lib/server";

/** Phase 5 API shapes (src/routes/databases.rs). Times are unix seconds. */

export interface DbStatus {
  /** missing = no PG_ADMIN_URL on the server, unreachable = URL found but the connection failed. */
  state: "ok" | "missing" | "unreachable";
  error: string | null;
  installCommand: string;
  pg: {
    version: string;
    listen: string;
    databases: number;
    sizeBytes: number;
    connections: number;
    maxConnections: number;
    cacheHit: number | null;
    sharedBuffers: string;
  } | null;
  backup: { lastAt: number | null; ok: number; total: number; sizeBytes: number; dir: string; dirBytes: number };
}

export interface DbItem {
  /** Unused; lets DataTable's `{ id?: … }` constraint accept the type. */
  id?: string;
  name: string;
  owner: string;
  sizeBytes: number | null;
  connections: number | null;
  extensions: string[];
  app: string | null;
  /** postgres://owner@host:port/name, no password. */
  url: string;
  createdAt: number;
  lastBackup: { at: number; ok: boolean; sizeBytes: number | null; error: string | null } | null;
}

export interface DbCreated {
  name: string;
  owner: string;
  host: string;
  port: number;
  password: string;
  url: string;
  extensions: string[];
  linkedApp: string | null;
  warnings: string[];
}

export const EXTENSIONS = ["pg_trgm", "uuid-ossp", "unaccent"] as const;

/** Same rule as the server: ^[a-z_][a-z0-9_]{0,62}$ */
export const IDENT = /^[a-z_][a-z0-9_]{0,62}$/;

export function fmtSize(bytes: number | null | undefined): string {
  if (bytes == null) return "—";
  if (bytes >= 2 ** 30) return `${(bytes / 2 ** 30).toFixed(1)} GB`;
  if (bytes >= 2 ** 20) return `${Math.round(bytes / 2 ** 20)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

const wibDay = (secs: number) => new Date(secs * 1000 + 7 * 3600_000).toISOString().slice(0, 10);

/** "today 02:00" or "17 Oct 02:00", in WIB. */
export function when(at: number): string {
  const time = formatWib(at, { hour: "2-digit", minute: "2-digit", hour12: false });
  return wibDay(at) === wibDay(Date.now() / 1000) ? `today ${time}` : `${formatWib(at, { day: "numeric", month: "short" })} ${time}`;
}

/** 32 characters from [A-Za-z0-9], unbiased (bytes >= 248 are skipped). */
export function randomPassword(): string {
  const A = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let out = "";
  while (out.length < 32) {
    for (const b of crypto.getRandomValues(new Uint8Array(64))) if (b < 248 && out.length < 32) out += A[b % 62];
  }
  return out;
}

/** The apps endpoint is phase 3's: accept `{data: [...]}` or a bare array. */
export function appNames(body: unknown): string[] {
  const list = Array.isArray(body) ? body : (body as { data?: unknown })?.data;
  if (!Array.isArray(list)) return [];
  return list.map((a) => (a as { name?: unknown })?.name).filter((n): n is string => typeof n === "string");
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
