/** Shapes of GET /api/metrics/* and /api/services (src/routes/metrics.rs). Bytes are bytes, rates bytes/s, times unix seconds. */

export interface Sample {
  ts: number;
  cpu: number;
  cpuMax: number;
  load1: number;
  load5: number;
  load15: number;
  memUsed: number;
  memCache: number;
  memTotal: number;
  swapUsed: number;
  swapTotal: number;
  diskUsed: number;
  diskTotal: number;
  diskRead: number;
  diskWrite: number;
  netRx: number;
  netTx: number;
}

export interface Current {
  sample: Sample | null;
  online: boolean;
  ageSecs?: number;
  cores?: number;
  iface?: string;
  uptimeSecs?: number;
  os?: string;
  peak?: { cpu: number; ts: number } | null;
  today?: { rx: number; tx: number; peakRx: number; peakTx: number };
}

export interface Series {
  range: string;
  from: number;
  step: number;
  slots: number;
  points: Sample[];
}

export interface Proc {
  id?: string; // unused; DataTable wants an optional id
  pid: number;
  name: string;
  app: string | null;
  cpu: number;
  memBytes: number;
  uptimeSecs: number;
}

export interface ServiceInfo {
  name: string;
  state: string;
  version: string | null;
}

export interface DiskUsage {
  at: number | null;
  entries: { key: string; label: string; bytes: number }[];
  otherBytes: number | null;
}

export interface Alert {
  key: string;
  level: "warning" | "critical";
  title: string;
  detail: string;
  short: string;
  href: string;
}

export interface Thresholds {
  diskWarn: number;
  diskStop: number;
  cpuPct: number;
  cpuMinutes: number;
  memWarn: number;
}

export interface AlertsResponse {
  alerts: Alert[];
  thresholds: Thresholds;
}

export const DEFAULT_THRESHOLDS: Thresholds = { diskWarn: 85, diskStop: 95, cpuPct: 80, cpuMinutes: 5, memWarn: 90 };
