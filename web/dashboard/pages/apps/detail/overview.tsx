import { useState } from "react";
import { api } from "../../../lib/api";
import { formatWib } from "../../../lib/server";
import { useAuth } from "../../../lib/auth";
import { useEventStream } from "../../../lib/useEventStream";
import { StatusBadge } from "../../../components/Badge";
import { Button, IconButton } from "../../../components/Button";
import { DataTable, type Column } from "../../../components/DataTable";
import { Toggle } from "../../../components/Input";
import { LogView } from "../../../components/LogView";
import { ConfirmDialog, Modal } from "../../../components/Modal";
import { useToast } from "../../../components/Toast";
import { CopyIcon } from "../../../components/icons";
import { DeployPanel } from "../../../components/DeployPanel";
import { appUrl, copy, fmtBytes, fmtDuration, took, useApi, webhookUrl, type AppDetailData, type Deploy } from "../../../components/appsApi";

interface DeployList {
  data: Deploy[];
  releasesOnDisk: number[];
  currentRelease: number | null;
}

function Card({ title, meta, children }: { title: string; meta?: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-3 rounded-xl border border-line bg-surface p-5">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-15 font-semibold">{title}</h2>
        {meta && <span className="truncate font-mono text-xs text-ink-3">{meta}</span>}
      </div>
      {children}
    </section>
  );
}

function ProcessCard({ app }: { app: AppDetailData }) {
  const p = app.process;
  const up = p?.activeState === "active" || p?.activeState === "reloading";
  const crashed = app.status === "crashed";
  const now = Date.now() / 1000;
  const items: [string, string, string?][] = [
    ["Status", p ? (p.subState || p.activeState) : "unknown", crashed ? "text-danger" : up ? "text-success" : "text-ink"],
    ["PID", p?.pid ? String(p.pid) : "—"],
    ["Uptime", up && p?.startedAt ? fmtDuration(Math.round(now - p.startedAt)) : "0s"],
    ["CPU", up && p?.cpuPct != null ? `${p.cpuPct}%` : "—"],
    ["Memory", up ? fmtBytes(p?.memBytes) : "—"],
    ["Restarts", String(p?.restarts ?? 0), (p?.restarts ?? 0) > 0 ? "text-danger" : undefined],
  ];
  return (
    <Card title="Process" meta={`systemd · ${app.unit}`}>
      <dl className="m-0 grid grid-cols-3 gap-3">
        {items.map(([k, v, c]) => (
          <div key={k} className="flex flex-col gap-0.5">
            <dt className="text-xs text-ink-3">{k}</dt>
            <dd className={`m-0 font-mono text-15 font-medium ${c ?? "text-ink"}`}>{v}</dd>
          </div>
        ))}
      </dl>
    </Card>
  );
}

function ReleaseCard({ app, behind }: { app: AppDetailData; behind?: string }) {
  const l = app.live;
  return (
    <Card title="Live release" meta={l ? `#${l.number}` : undefined}>
      {l ? (
        <div className="flex flex-col gap-1">
          <span className="text-sm">
            <span className="font-mono">{l.commit}</span> {l.message}
          </span>
          <span className="text-13 text-ink-3">
            {l.trigger === "webhook" ? `push to ${l.branch}` : (l.actor ?? l.trigger)} · {l.finishedAt ? formatWib(l.finishedAt) : ""} · took {took(l)}
            {l.release !== l.number ? ` · release #${l.release}` : ""}
          </span>
        </div>
      ) : (
        <span className="text-sm text-ink-2">No release yet. The first deploy puts one live.</span>
      )}
      {behind && (
        <div className="flex items-center gap-2 text-13 text-warning">
          <span className="h-1.5 w-1.5 rounded-full bg-warning" aria-hidden="true" />
          {behind}
        </div>
      )}
    </Card>
  );
}

function AutoDeployCard({ app, reload }: { app: AppDetailData; reload: () => void }) {
  const toast = useToast();
  const { publicUrl } = useAuth();
  const [saving, setSaving] = useState(false);
  const url = webhookUrl(app.webhookPath, publicUrl);
  const set = async (on: boolean) => {
    setSaving(true);
    try {
      await api(appUrl(app.name), { method: "PUT", body: { autoDeploy: on } });
      toast({ title: on ? "Auto-deploy on" : "Auto-deploy off", body: on ? `Pushes to ${app.branch} start a deploy.` : "Pushes are ignored.", tone: "success" });
      reload();
    } catch (e) {
      toast({ title: "Not saved", body: (e as Error).message, tone: "error" });
    } finally {
      setSaving(false);
    }
  };
  return (
    <section className="flex flex-col gap-3 rounded-xl border border-line bg-surface p-5">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-15 font-semibold">Auto-deploy</h2>
        <Toggle checked={app.autoDeploy} disabled={saving} onChange={set} label={<span className="text-13">On push to {app.branch}</span>} />
      </div>
      <p className="text-13 leading-5 text-ink-2">Add this webhook in GitHub › Settings › Webhooks, content type JSON, with the webhook secret from Settings.</p>
      <div className="flex h-11 items-center gap-2 rounded-lg border border-line bg-sunken pl-3 pr-0.5 font-mono text-xs">
        <span className="min-w-0 flex-1 truncate">{url}</span>
        <IconButton label="Copy webhook URL" variant="ghost" onClick={() => copy(url)?.then(() => toast({ title: "Webhook URL copied" }))}>
          <CopyIcon size={15} />
        </IconButton>
      </div>
    </section>
  );
}

function DeployLogModal({ app, deploy, onClose }: { app: string; deploy: Deploy | null; onClose: () => void }) {
  const { lines, done } = useEventStream(deploy ? `${appUrl(app)}/deploys/${deploy.id}/log` : null);
  return (
    <Modal open={!!deploy} onClose={onClose} size="xl" title={deploy ? `Deploy #${deploy.number} log` : ""} description={deploy?.error ?? undefined}>
      <LogView title={`Log · deploy #${deploy?.number ?? ""}`} lines={lines.map((l) => ({ text: l.text }))} live={!done} height={520} />
    </Modal>
  );
}

function ProcessLog({ app }: { app: string }) {
  const { lines, done } = useEventStream(`${appUrl(app)}/logs`);
  return (
    <LogView
      title={`Process log · journalctl -u app-${app}`}
      lines={lines.map((l) => ({ text: l.text }))}
      live={!done}
      height={320}
    />
  );
}

const canRollback = (d: Deploy, list: DeployList | undefined) =>
  d.status === "success" && d.trigger !== "rollback" && d.release != null && d.release !== list?.currentRelease && !!list?.releasesOnDisk.includes(d.release);

/** AppDetail.dc.html: running / deploying / failed / crashed. */
export function AppOverviewTab({ app, reload }: { app: AppDetailData; reload: () => void }) {
  const toast = useToast();
  const deploying = app.status === "deploying";
  const { data: list, reload: reloadList } = useApi<DeployList>(`${appUrl(app.name)}/deploys?limit=20`, deploying ? 3000 : 10000);
  const deploys = list?.data ?? [];
  const last = deploys[0];
  const [logOf, setLogOf] = useState<Deploy | null>(null);
  const [rollTo, setRollTo] = useState<Deploy | null>(null);
  const [rolling, setRolling] = useState(false);

  // Pipeline: the running deploy, else the newest one if it failed (until the next success).
  const failedLast = !deploying && last && (last.status === "failed" || last.status === "rolled_back") ? last : undefined;
  const panelId = app.activeDeploy?.id ?? failedLast?.id ?? null;

  const previous = deploys.find((d) => canRollback(d, list));
  const doRollback = async () => {
    if (!rollTo) return;
    setRolling(true);
    try {
      await api(`${appUrl(app.name)}/deploys/${rollTo.id}/rollback`, { method: "POST" });
      toast({ title: `Rolling back to #${rollTo.release}`, body: "Switch, restart and health check run now." });
      setRollTo(null);
      reload();
      reloadList();
    } catch (e) {
      toast({ title: "Rollback not started", body: (e as Error).message, tone: "error" });
    } finally {
      setRolling(false);
    }
  };

  const behind = deploying
    ? `#${app.activeDeploy?.number} is building, swaps in when healthy`
    : failedLast && app.live
      ? `#${failedLast.number}${failedLast.commit ? ` (${failedLast.commit})` : ""} ${failedLast.status === "rolled_back" ? "was rolled back" : "failed"}`
      : undefined;

  const columns: Column<Deploy>[] = [
    { key: "number", header: "#", mono: true, render: (d) => `#${d.number}` },
    { key: "status", header: "Status", render: (d) => <StatusBadge kind="deploy" status={d.status} /> },
    {
      key: "commit",
      header: "Commit",
      render: (d) => (
        <span className="block max-w-[420px] truncate text-13">
          <span className="font-mono">{d.commit ?? "—"}</span> {d.trigger === "rollback" ? `roll back to #${d.release}` : d.message}
        </span>
      ),
    },
    { key: "trigger", header: "Trigger", render: (d) => <span className="text-13 text-ink-2">{d.trigger === "webhook" ? `push · ${d.branch}` : d.trigger === "manual" ? (d.actor ?? "manual") : d.trigger}</span> },
    { key: "took", header: "Took", mono: true, align: "right", render: (d) => took(d) },
    { key: "when", header: "When", render: (d) => <span className="whitespace-nowrap text-13 text-ink-2">{formatWib(d.createdAt)}</span> },
    {
      key: "act",
      header: <span className="sr-only">Actions</span>,
      align: "right",
      render: (d) => (
        <span className="inline-flex gap-2">
          <Button size="sm" variant="subtle" onClick={() => setLogOf(d)}>
            Log
          </Button>
          {canRollback(d, list) && (
            <Button size="sm" variant="secondary" onClick={() => setRollTo(d)}>
              Roll back
            </Button>
          )}
        </span>
      ),
    },
  ];

  return (
    <div className="flex flex-col gap-6">
      {failedLast && app.status !== "crashed" && (
        <div role="alert" className="flex flex-wrap items-center gap-3 rounded-xl bg-danger-bg py-3 pl-4 pr-3 text-danger-ink">
          <span className="h-2 w-2 shrink-0 rounded-full bg-danger" aria-hidden="true" />
          <div className="min-w-0 flex-1 text-sm">
            <strong>
              Deploy #{failedLast.number} {failedLast.status === "rolled_back" ? "was rolled back." : "failed."}
            </strong>{" "}
            {failedLast.error}{" "}
            {app.live
              ? `Release #${app.live.release} (${app.live.commit}) is ${failedLast.status === "rolled_back" ? "live again" : "still running, nothing was restarted"}.`
              : "No release is live yet."}
          </div>
          <Button variant="secondary" onClick={() => document.getElementById("deploy-log")?.scrollIntoView({ behavior: "smooth" })}>
            Jump to error
          </Button>
        </div>
      )}
      {app.status === "crashed" && (
        <div role="alert" className="flex flex-wrap items-center gap-3 rounded-xl bg-danger-bg py-3 pl-4 pr-3 text-danger-ink">
          <span className="h-2 w-2 shrink-0 rounded-full bg-danger" aria-hidden="true" />
          <div className="min-w-0 flex-1 text-sm">
            <strong>{app.name} is down.</strong> systemd restarted it {app.process?.restarts ?? 0} times, then stopped ({app.process?.result || "failed"}). The
            process log below shows the last error.{app.domain ? ` nginx returns 502 for ${app.domain}.` : ""}
          </div>
          <Button variant="secondary" href={`#/apps/${app.name}/env`}>
            Open Environment
          </Button>
          {previous && <Button onClick={() => setRollTo(previous)}>Roll back to #{previous.release}</Button>}
        </div>
      )}

      <div className="grid grid-cols-[repeat(auto-fit,minmax(280px,1fr))] gap-6">
        <ProcessCard app={app} />
        <ReleaseCard app={app} behind={behind} />
        <AutoDeployCard app={app} reload={reload} />
      </div>

      {panelId && (
        <DeployPanel
          key={panelId}
          app={app.name}
          deployId={panelId}
          onFinish={() => {
            reload();
            reloadList();
          }}
        />
      )}

      <section className="overflow-hidden rounded-xl border border-line bg-surface">
        <div className="flex flex-wrap items-baseline justify-between gap-3 border-b border-line px-6 py-4">
          <h2 className="text-17 font-semibold">Deployments</h2>
          <span className="text-13 text-ink-3">Last {app.keepReleases} releases are kept for rollback</span>
        </div>
        <DataTable columns={columns} data={deploys} loading={!list} rowKey={(d) => d.id} framed={false} minWidth={860} label="Deployments" emptyMessage="No deploys yet" />
      </section>

      {!panelId && <ProcessLog key={app.name} app={app.name} />}

      <DeployLogModal app={app.name} deploy={logOf} onClose={() => setLogOf(null)} />
      <ConfirmDialog
        open={!!rollTo}
        onClose={() => setRollTo(null)}
        onConfirm={doRollback}
        busy={rolling}
        tone="primary"
        title={`Roll back ${app.name} to #${rollTo?.release ?? ""}?`}
        confirmLabel={`Roll back to #${rollTo?.release ?? ""}`}
        cancelLabel="Keep current"
      >
        <code className="font-mono">current</code> switches to release <strong>#{rollTo?.release}</strong> ({rollTo?.commit} {rollTo?.message}), then the process restarts and must
        pass the health check. If it doesn&apos;t, release #{app.live?.release ?? "—"} comes back.
      </ConfirmDialog>
    </div>
  );
}
