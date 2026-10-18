import type { Tone } from "./Badge";

/** Shapes from src/routes/nginx.rs (camelCase JSON). */
export interface NginxSite {
  id: number;
  domain: string;
  aliases: string[];
  kind: "proxy" | "static" | "redirect";
  appId: number | null;
  appName: string | null;
  upstream: string | null;
  rootPath: string | null;
  redirectUrl: string | null;
  maxUploadMb: number;
  websocket: boolean;
  gzip: boolean;
  cacheAssets: boolean;
  customDirectives: string;
  enabled: boolean;
  ssl: boolean;
  forceHttps: boolean;
  sslEmail: string | null;
  sslExpiresAt: number | null;
  sslDaysLeft: number | null;
  certReady: boolean;
  file: string;
  updatedAt: number;
}

/** What the editor sends (POST /sites, PUT /sites/{id}, POST /sites/preview). */
export interface NginxForm {
  id?: number;
  domain: string;
  aliases: string[];
  kind: NginxSite["kind"];
  appId: number | null;
  rootPath: string;
  redirectUrl: string;
  maxUploadMb: number;
  websocket: boolean;
  gzip: boolean;
  cacheAssets: boolean;
  customDirectives: string;
  ssl: boolean;
  forceHttps: boolean;
  sslEmail: string;
}

export interface NginxAppOption {
  id: number;
  name: string;
  port: number | null;
}

export interface NginxTest {
  ok: boolean;
  output: string;
  line?: number | null;
}

export interface NginxStatus {
  active: boolean;
  version: string | null;
  lastTest: { at: number; ok: boolean; output: string } | null;
  sites: { total: number; enabled: number };
  certs: { total: number; expiring: { id: number; domain: string; daysLeft: number }[] };
  publicIp: string;
  dir: string;
}

export interface NginxDns {
  serverIp: string;
  results: { name: string; ips: string[]; ok: boolean; error: string | null }[];
}

export const emptyForm: NginxForm = {
  domain: "",
  aliases: [],
  kind: "proxy",
  appId: null,
  rootPath: "",
  redirectUrl: "",
  maxUploadMb: 10,
  websocket: true,
  gzip: true,
  cacheAssets: false,
  customDirectives: "",
  ssl: false,
  forceHttps: true,
  sslEmail: "",
};

export function formFromSite(s: NginxSite): NginxForm {
  return {
    id: s.id,
    domain: s.domain,
    aliases: s.aliases,
    kind: s.kind,
    appId: s.appId,
    rootPath: s.rootPath ?? "",
    redirectUrl: s.redirectUrl ?? "",
    maxUploadMb: s.maxUploadMb,
    websocket: s.websocket,
    gzip: s.gzip,
    cacheAssets: s.cacheAssets,
    customDirectives: s.customDirectives,
    ssl: s.ssl,
    forceHttps: s.forceHttps,
    sslEmail: s.sslEmail ?? "",
  };
}

export const KIND_LABEL: Record<NginxSite["kind"], string> = { proxy: "Proxy", static: "Static", redirect: "Redirect" };

/** Certificate state as a status label: never color alone. */
export function sslLabel(s: Pick<NginxSite, "ssl" | "certReady" | "sslDaysLeft">): { tone: Tone; text: string } {
  if (!s.certReady || s.sslDaysLeft == null) return s.ssl ? { tone: "info", text: "Not issued yet" } : { tone: "neutral", text: "No SSL" };
  if (s.sslDaysLeft < 0) return { tone: "danger", text: "Expired" };
  if (s.sslDaysLeft <= 30) return { tone: "warning", text: s.sslDaysLeft === 0 ? "Expires today" : `Expires in ${s.sslDaysLeft} ${s.sslDaysLeft === 1 ? "day" : "days"}` };
  return { tone: "success", text: `Valid · ${s.sslDaysLeft} days` };
}

/** "radio.example.com, www.example.com" → ["radio.example.com", …] */
export function splitAliases(text: string): string[] {
  return text
    .split(/[,\s]+/)
    .map((a) => a.trim().toLowerCase())
    .filter(Boolean);
}

/** `2026/10/18 15:40:53 [emerg] 35#35: unknown directive "x" in /etc/…/a.conf:31` → `unknown directive "x"` */
export function emergMessage(output: string): string | null {
  const l = output.split("\n").find((x) => /\[(emerg|crit|alert)\]/.test(x));
  if (!l) return null;
  return l
    .replace(/^.*?\[(emerg|crit|alert)\]\s*(\d+#\d+:\s*)?/, "")
    .replace(/\s+in\s+\S+:\d+\s*$/, "")
    .trim();
}
