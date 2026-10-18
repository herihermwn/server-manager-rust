import { formatWib } from "../../lib/server";
import type { Thresholds } from "./types";

const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** Pick the unit for `n` (1024-based) and the value in it. */
function scaled(n: number): [number, number] {
  let i = 0;
  let v = Math.max(0, n);
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return [v, i];
}

function digits(v: number, i: number) {
  return v >= 100 || i === 0 ? v.toFixed(0) : v.toFixed(1);
}

/** 2254857830 → ["2.1", "GB"]. */
export function bytesParts(n: number): [string, string] {
  const [v, i] = scaled(n);
  return [digits(v, i), UNITS[i] ?? "B"];
}

/** "2.1 GB", "410 MB". */
export function bytes(n: number) {
  return bytesParts(n).join(" ");
}

/** Bytes per second → ["2.4", "MB/s"]. */
export function rateParts(n: number): [string, string] {
  const [v, i] = scaled(n);
  return [digits(v, i), `${UNITS[i] ?? "B"}/s`];
}

export function rate(n: number) {
  return rateParts(n).join(" ");
}

/** Largest "nice" number (1, 2, 5 × 10^k, in the unit of `max`) at or above `max`: the top of a chart axis. */
export function niceCeil(max: number): number {
  if (!(max > 0)) return 1024;
  let unit = 1;
  while (max / unit >= 1024) unit *= 1024;
  const v = max / unit;
  const exp = 10 ** Math.floor(Math.log10(v));
  const m = v / exp;
  return (m <= 1 ? 1 : m <= 2 ? 2 : m <= 5 ? 5 : 10) * exp * unit;
}

/** 90 → "1 min", 7200 → "2 h", 2e6 → "23 d". */
export function duration(secs: number) {
  if (secs < 90) return `${Math.max(0, Math.round(secs))} s`;
  if (secs < 5400) return `${Math.round(secs / 60)} min`;
  if (secs < 129_600) return `${Math.round(secs / 3600)} h`;
  return `${Math.round(secs / 86_400)} d`;
}

/** "13:04" in WIB. */
export function clock(ts: number, seconds = false) {
  return formatWib(ts, { hour: "2-digit", minute: "2-digit", ...(seconds ? { second: "2-digit" } : {}), hour12: false });
}

/** "18 Oct" in WIB. */
export function day(ts: number) {
  return formatWib(ts, { day: "numeric", month: "short" });
}

/** "today 13:04", "yesterday 21:44", "16 Oct 03:00" (WIB). Accepts seconds or milliseconds. */
export function whenWib(ts: number) {
  const secs = ts > 1e11 ? Math.floor(ts / 1000) : ts;
  const d = (t: number) => formatWib(t, { year: "numeric", month: "2-digit", day: "2-digit" });
  const nowSecs = Math.floor(Date.now() / 1000);
  if (d(secs) === d(nowSecs)) return `today ${clock(secs)}`;
  if (d(secs) === d(nowSecs - 86_400)) return `yesterday ${clock(secs)}`;
  return `${day(secs)} ${clock(secs)}`;
}

export type Level = "ok" | "warn" | "bad";

export const pct = (used: number, total: number) => (total > 0 ? (used / total) * 100 : 0);

/** Level of each reading against the alert thresholds (same numbers the alerts use). */
export const levels = {
  cpu: (v: number, t: Thresholds): Level => (v >= t.cpuPct ? "bad" : "ok"),
  mem: (v: number, t: Thresholds): Level => (v >= t.memWarn ? "bad" : v >= t.memWarn - 10 ? "warn" : "ok"),
  disk: (v: number, t: Thresholds): Level => (v >= t.diskStop ? "bad" : v >= t.diskWarn ? "warn" : "ok"),
  load: (load1: number, cores: number): Level => (load1 > cores * 1.5 ? "bad" : load1 > cores ? "warn" : "ok"),
};
