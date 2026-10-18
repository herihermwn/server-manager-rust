import { Banner, EmptyState, ErrorState, Skeleton } from "../../components/Feedback";
import { Button } from "../../components/Button";
import { PageHeader } from "../../components/PageHeader";
import { StatusBadge } from "../../components/Badge";
import { Section } from "../../components/Card";
import { DataTable, type Column } from "../../components/DataTable";
import { PlusIcon } from "../../components/icons";
import { UsageKpiStrip, type UsageKpi } from "../../components/UsageKpiStrip";
import { usePoll } from "../../components/usePoll";
import { SERVER } from "../../lib/server";
import { bytes, bytesParts, clock, duration, levels, pct, rate, rateParts, whenWib } from "../usage/format";
import { DEFAULT_THRESHOLDS, type AlertsResponse, type Current, type ServiceInfo } from "../usage/types";

// GET /api/apps belongs to phase 3: { data: [...] }. A 404 means it is not built yet.
interface AppRow {
  id?: string; // unused; DataTable wants an optional id
  name: string;
  runtime: string;
  status: "running" | "stopped" | "crashed" | "deploying";
  port: number | null;
  domain: string | null;
  branch: string;
  commit: string;
  lastDeploy: { id: number; status: string; finishedAt: number | null } | null;
}

interface ActivityRow {
  id: number;
  at: number;
  actor: string;
  action: string;
  target: string;
  detail: string;
}

const RUNTIME: Record<string, string> = { rust: "Rust", bun: "Bun", node: "Node.js", static: "Static" };
const ACTION_LABEL: Record<string, string> = {
  "auth.login": "Signed in",
  "auth.logout": "Signed out",
  "auth.password": "Changed the password",
  "auth.lockout": "Locked sign-in after failed attempts",
  "auth.create_admin": "Created the admin account",
};
const VERB: Record<string, string> = {
  create: "Created",
  update: "Updated",
  delete: "Deleted",
  start: "Started",
  stop: "Stopped",
  restart: "Restarted",
  deploy: "Deployed",
  rollback: "Rolled back",
  save: "Saved",
  backup: "Backed up",
  reveal: "Revealed",
  import: "Imported",
};

/** "app.create" + "radio-api-rs" → "Created app radio-api-rs". */
function describe(a: ActivityRow) {
  if (ACTION_LABEL[a.action]) return ACTION_LABEL[a.action] + (a.target ? ` · ${a.target}` : "");
  const [area = "", verb = ""] = a.action.split(".");
  const v = VERB[verb] ?? verb.replace(/_/g, " ");
  const text = `${v.charAt(0).toUpperCase()}${v.slice(1)} ${area.replace(/_/g, " ")}`.trim();
  return a.target ? `${text} ${a.target}` : text;
}

function dotFor(a: ActivityRow) {
  if (/fail|lockout|error|delete/.test(a.action) || /fail|error/i.test(a.detail)) return "bg-danger";
  if (/update|save|env|import/.test(a.action)) return "bg-info";
  return "bg-success";
}

const ACTION_BUTTON: Record<string, string> = { disk: "See what uses disk", cpu: "See usage", memory: "See usage" };

export function Overview() {
  const cur = usePoll<Current>("/api/metrics/current", 10_000);
  const alerts = usePoll<AlertsResponse>("/api/metrics/alerts", 15_000);
  const services = usePoll<ServiceInfo[]>("/api/services", 15_000);
  const apps = usePoll<{ data: AppRow[] }>("/api/apps", 15_000);
  const activity = usePoll<ActivityRow[]>("/api/activity?limit=6", 30_000);

  const c = cur.data;
  const s = c?.sample ?? null;
  const t = alerts.data?.thresholds ?? DEFAULT_THRESHOLDS;
  const cores = c?.cores ?? 1;
  const appList = apps.data?.data ?? [];
  const crashed = appList.filter((a) => a.status === "crashed");
  const banners = c?.online === false ? [] : (alerts.data?.alerts ?? []);

  const kpis: UsageKpi[] = s
    ? (() => {
        const memPct = pct(s.memUsed, s.memTotal);
        const diskPct = pct(s.diskUsed, s.diskTotal);
        const [memV, memU] = bytesParts(s.memUsed);
        const [rxV, rxU] = rateParts(s.netRx);
        const diskLevel = levels.disk(diskPct, t);
        const load = levels.load(s.load1, cores);
        return [
          {
            label: "CPU",
            value: Math.round(s.cpu),
            unit: `% · ${cores} ${cores === 1 ? "core" : "cores"}`,
            pct: s.cpu,
            meta: c?.peak ? `Peak ${Math.round(c.peak.cpu)}% at ${clock(c.peak.ts)}` : "Last hour",
            level: levels.cpu(s.cpu, t),
            href: "#/usage",
          },
          {
            label: "Memory",
            value: memV,
            unit: `/ ${bytes(s.memTotal)}`,
            pct: memPct,
            meta: `${bytes(s.memTotal - s.memUsed)} free · ${s.swapTotal > 0 ? `swap ${bytes(s.swapUsed)} in use` : "no swap"}`,
            level: levels.mem(memPct, t),
            href: "#/usage",
          },
          {
            label: "Disk",
            value: bytesParts(s.diskUsed)[0],
            unit: `${bytesParts(s.diskUsed)[1]} / ${bytes(s.diskTotal)}`,
            pct: diskPct,
            meta: `${bytes(s.diskTotal - s.diskUsed)} free${diskPct >= t.diskStop ? " · deploys blocked" : ""}`,
            level: diskLevel,
            href: "#/usage",
          },
          {
            label: "Network",
            value: rxV,
            unit: `${rxU} in`,
            meta: `Out ${rate(s.netTx)}${c?.today ? ` · ${bytes(c.today.rx + c.today.tx)} today` : ""}`,
            href: "#/usage",
          },
          {
            label: "Load",
            value: s.load1.toFixed(2),
            unit: "1 min",
            pct: Math.min(100, (s.load1 / cores) * 100),
            meta: `5 min ${s.load5.toFixed(2)} · 15 min ${s.load15.toFixed(2)}`,
            level: load,
            href: "#/usage",
          },
        ];
      })()
    : ["CPU", "Memory", "Disk", "Network", "Load"].map((label) => ({ label, value: "–", pct: label === "Network" ? undefined : 0, href: "#/usage" }));

  const subtitle = (
    <>
      <span className="font-mono">
        {SERVER.host}
      </span>
      {c?.os ? ` · ${c.os}` : ""}
      {c?.cores ? ` · ${c.cores} vCPU` : ""}
      {s ? ` · ${bytes(s.memTotal)}` : ""}
      {c?.uptimeSecs ? ` · up ${duration(c.uptimeSecs)}` : ""}
    </>
  );

  const appCols: Column<AppRow>[] = [
    {
      key: "name",
      header: "App",
      render: (a) => (
        <span className="flex flex-col py-1">
          <span className="font-semibold text-ink">{a.name}</span>
          <span className="text-xs text-ink-3">
            {[RUNTIME[a.runtime] ?? a.runtime, "systemd", a.port ? `:${a.port}` : ""].filter(Boolean).join(" · ")}
          </span>
        </span>
      ),
    },
    { key: "status", header: "Status", render: (a) => <StatusBadge status={a.status} /> },
    { key: "domain", header: "Domain", mono: true, render: (a) => a.domain ?? "" },
    {
      key: "last",
      header: "Last deploy",
      render: (a) =>
        a.lastDeploy ? (
          <span className="whitespace-nowrap text-13 text-ink-2">
            #{a.lastDeploy.id}
            {a.lastDeploy.finishedAt ? ` · ${whenWib(a.lastDeploy.finishedAt)}` : " · running"}
          </span>
        ) : (
          <span className="text-13 text-ink-3">Never deployed</span>
        ),
    },
  ];

  const noApps = apps.error?.status === 404 || (apps.loaded && !apps.error && appList.length === 0);

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Overview"
        subtitle={subtitle}
        actions={
          <>
            <Button variant="subtle" href="#/usage">
              View usage
            </Button>
            <Button href="#/apps/new" icon={<PlusIcon />}>
              New app
            </Button>
          </>
        }
      />

      {(banners.length > 0 || crashed.length > 0) && (
        <div className="flex flex-col gap-2">
          {banners.map((a) => (
            <Banner
              key={a.key}
              tone={a.level === "critical" ? "error" : "warning"}
              title={a.title}
              action={ACTION_BUTTON[a.key] ? { label: ACTION_BUTTON[a.key] ?? "", href: a.href } : undefined}
            >
              {a.detail}
            </Banner>
          ))}
          {crashed.slice(0, 3).map((a) => (
            <Banner key={a.name} tone="error" title={`${a.name} crashed.`} action={{ label: `Open ${a.name}`, href: `#/apps/${a.name}` }}>
              systemd stopped restarting it. Open it to see the log.
            </Banner>
          ))}
        </div>
      )}

      {cur.error && !cur.data ? (
        <ErrorState title="Could not load server metrics" description={cur.error.message} onRetry={cur.reload} />
      ) : (
        <UsageKpiStrip label="Server resources" items={kpis} loading={!cur.loaded} />
      )}
      {c && !s && <p className="-mt-3 text-13 text-ink-2">Waiting for the first sample. It is recorded about 10 s after the manager starts.</p>}

      <div className="flex flex-wrap items-start gap-6">
        <div className="min-w-0 flex-[2_1_560px]">
          <Section
            flush
            title={
              <>
                Apps {apps.data && <span className="font-mono text-13 font-medium text-ink-3">{appList.length}</span>}
              </>
            }
            action={
              <Button variant="subtle" href="#/apps">
                All apps
              </Button>
            }
          >
            {apps.error && apps.error.status !== 404 && !apps.data ? (
              <div className="p-5">
                <ErrorState title="Could not load apps" description={apps.error.message} onRetry={apps.reload} />
              </div>
            ) : noApps ? (
              <EmptyState
                title="No apps yet"
                description="Add one from a git link. The first deploy runs right after the clone."
                action={
                  <Button href="#/apps/new" icon={<PlusIcon />}>
                    New app
                  </Button>
                }
              />
            ) : (
              <DataTable<AppRow> framed={false} label="Apps" columns={appCols} data={appList} loading={!apps.loaded} skeletonRows={4} rowKey={(a) => a.name} rowHref={(a) => `#/apps/${a.name}`} />
            )}
          </Section>
        </div>

        <div className="flex min-w-0 flex-[1_1_320px] flex-col gap-6">
          <Section flush title="Services">
            {services.error && !services.data ? (
              <div className="p-5">
                <ErrorState title="Could not load services" description={services.error.message} onRetry={services.reload} />
              </div>
            ) : !services.data ? (
              <ul aria-busy="true" aria-label="Loading services">
                {[0, 1, 2, 3].map((i) => (
                  <li key={i} className="flex h-[52px] items-center gap-3 border-t border-line-soft px-5 first:border-t-0">
                    <Skeleton className="h-3 w-28" />
                  </li>
                ))}
              </ul>
            ) : (
              <ul>
                {services.data.map((sv) => (
                  <li key={sv.name} className="flex min-h-[52px] items-center justify-between gap-3 border-t border-line-soft px-5 py-2 first:border-t-0">
                    <div className="flex min-w-0 flex-col">
                      <span className="font-mono text-sm font-medium text-ink">{sv.name}</span>
                      <span className="text-xs text-ink-3">{sv.version ? `v${sv.version}` : "Not installed"}</span>
                    </div>
                    <StatusBadge kind="service" status={sv.state} />
                  </li>
                ))}
              </ul>
            )}
          </Section>

          <Section flush title="Recent activity">
            {activity.error && !activity.data ? (
              <div className="p-5">
                <ErrorState title="Could not load activity" description={activity.error.message} onRetry={activity.reload} />
              </div>
            ) : !activity.data ? (
              <ol aria-busy="true" aria-label="Loading activity" className="flex flex-col gap-4 px-5 py-4">
                {[0, 1, 2, 3].map((i) => (
                  <li key={i}>
                    <Skeleton className="h-3 w-4/5" />
                  </li>
                ))}
              </ol>
            ) : activity.data.length === 0 ? (
              <EmptyState title="Nothing yet" description="Sign-ins, deploys and other changes show up here." />
            ) : (
              <ol className="flex flex-col px-5 py-2">
                {activity.data.map((a) => (
                  <li key={a.id} className="flex gap-3 py-2.5">
                    <span className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${dotFor(a)}`} aria-hidden="true" />
                    <div className="flex min-w-0 flex-col gap-0.5">
                      <span className="text-sm text-ink">{describe(a)}</span>
                      <span className="font-mono text-xs text-ink-3">
                        {whenWib(a.at)} · {a.actor}
                        {a.detail ? ` · ${a.detail}` : ""}
                      </span>
                    </div>
                  </li>
                ))}
              </ol>
            )}
          </Section>
        </div>
      </div>
    </div>
  );
}
