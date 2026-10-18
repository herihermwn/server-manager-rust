import { useEffect, useState, type ReactNode } from "react";
import { api } from "../../../lib/api";
import { navigate } from "../../../lib/router";
import { useAuth } from "../../../lib/auth";
import { Button } from "../../../components/Button";
import { InlineAlert } from "../../../components/Feedback";
import { Checkbox, TextField, Textarea } from "../../../components/Input";
import { ConfirmDialog } from "../../../components/Modal";
import { useToast } from "../../../components/Toast";
import { appUrl, webhookUrl, type AppDetailData } from "../../../components/appsApi";

interface Form {
  repo: string;
  branch: string;
  keepReleases: string;
  sharedPaths: string;
  buildCmd: string;
  beforeRestartCmd: string;
  startCmd: string;
  port: string;
  healthPath: string;
  timeoutMin: string;
  runAs: string;
}

const fromApp = (a: AppDetailData): Form => ({
  repo: a.repo,
  branch: a.branch,
  keepReleases: String(a.keepReleases),
  sharedPaths: a.sharedPaths.join(", "),
  buildCmd: a.buildCmd,
  beforeRestartCmd: a.beforeRestartCmd,
  startCmd: a.startCmd,
  port: a.port == null ? "" : String(a.port),
  healthPath: a.healthPath,
  timeoutMin: String(Math.round(a.buildTimeoutSecs / 60)),
  runAs: a.runAs,
});

function Group({ first, title, desc, children }: { first?: boolean; title: string; desc: string; children: ReactNode }) {
  return (
    <div className={`grid gap-4 px-6 py-6 lg:grid-cols-[260px_1fr] lg:gap-8 ${first ? "" : "border-t border-line-soft"}`}>
      <div className="flex flex-col gap-1">
        <h2 className="text-15 font-semibold">{title}</h2>
        <p className="text-13 leading-5 text-ink-2">{desc}</p>
      </div>
      <div className="grid min-w-0 gap-4 sm:grid-cols-2">{children}</div>
    </div>
  );
}

function ReadOnly({ label, value, helper, wide }: { label: string; value: string; helper?: string; wide?: boolean }) {
  return (
    <div className={`flex min-w-0 flex-col gap-1.5 ${wide ? "sm:col-span-2" : ""}`}>
      <span className="text-13 font-semibold">{label}</span>
      <div className="flex h-11 items-center truncate rounded-lg border border-line bg-paper px-3 font-mono text-sm text-ink-2">{value}</div>
      {helper && <span className="text-xs text-ink-3">{helper}</span>}
    </div>
  );
}

/** AppSettings.dc.html (default / delete). */
export function AppSettingsTab({ app, reload }: { app: AppDetailData; reload: () => void }) {
  const toast = useToast();
  const { publicUrl } = useAuth();
  const [f, setF] = useState<Form>(() => fromApp(app));
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState("");
  const [stopping, setStopping] = useState(false);
  const [del, setDel] = useState(false);
  const [deleteFolder, setDeleteFolder] = useState(true);
  const [deleting, setDeleting] = useState(false);
  const base = fromApp(app);
  const baseKey = JSON.stringify(base);
  const dirty = (Object.keys(base) as (keyof Form)[]).filter((k) => f[k] !== base[k]);

  // Pick up server changes (e.g. auto-deploy toggled elsewhere) while nothing is edited here.
  useEffect(() => {
    if (dirty.length === 0) setF(fromApp(app));
  }, [baseKey]);

  const set = (k: keyof Form) => (e: { target: { value: string } }) => setF((x) => ({ ...x, [k]: e.target.value }));

  const save = async () => {
    setSaving(true);
    setErr("");
    const body: Record<string, unknown> = {};
    for (const k of dirty) {
      if (k === "keepReleases") body.keepReleases = Number(f.keepReleases);
      else if (k === "port") body.port = Number(f.port);
      else if (k === "timeoutMin") body.buildTimeoutSecs = Math.round(Number(f.timeoutMin) * 60);
      else if (k === "sharedPaths") body.sharedPaths = f.sharedPaths.split(",").map((s) => s.trim()).filter(Boolean);
      else body[k] = f[k];
    }
    try {
      const r = await api<{ data: { unitWarning: string | null } }>(appUrl(app.name), { method: "PUT", body });
      toast({ title: "Settings saved", body: r.data.unitWarning ? `Unit not reloaded: ${r.data.unitWarning}` : "Commands apply on the next deploy.", tone: "success" });
      reload();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setSaving(false);
    }
  };

  const stopped = app.status === "stopped" || app.status === "crashed";
  const toggleRun = async () => {
    setStopping(true);
    try {
      await api(`${appUrl(app.name)}/${stopped ? "start" : "stop"}`, { method: "POST" });
      toast({ title: `${app.name} ${stopped ? "started" : "stopped"}`, tone: "success" });
      reload();
    } catch (e) {
      toast({ title: stopped ? "Could not start" : "Could not stop", body: (e as Error).message, tone: "error" });
    } finally {
      setStopping(false);
    }
  };

  const remove = async () => {
    setDeleting(true);
    try {
      const r = await api<{ data: { note: string } }>(appUrl(app.name), { method: "DELETE", body: { deleteFolder } });
      toast({ title: `${app.name} deleted`, body: r.data.note, tone: "success" });
      navigate("/apps");
    } catch (e) {
      toast({ title: "Not deleted", body: (e as Error).message, tone: "error" });
      setDeleting(false);
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <section className="overflow-hidden rounded-xl border border-line bg-surface">
        <Group first title="Source" desc="Where the code comes from. A new repository link is used from the next deploy.">
          <TextField label="Repository" mono value={f.repo} onChange={set("repo")} wrapperClassName="sm:col-span-2" />
          <TextField label="Branch" mono value={f.branch} onChange={set("branch")} />
          <ReadOnly label="Access" value="Server deploy key · ed25519" helper="Change in Settings › Git access." />
        </Group>
        <Group title="Folder & releases" desc="Each deploy builds into releases/<n>, then current points to it. Shared paths survive deploys.">
          <ReadOnly label="App folder" value={app.path} wide helper="Fixed after creation. To move it, delete and add the app again." />
          <TextField label="Releases to keep" mono inputMode="numeric" value={f.keepReleases} onChange={set("keepReleases")} helper="Older ones are removed after a successful deploy." />
          <TextField label="Shared paths" mono value={f.sharedPaths} onChange={set("sharedPaths")} helper="Comma separated, linked into every release. A trailing / makes a folder." />
        </Group>
        <Group title="Build & run" desc="Commands run in the release folder with .env loaded. A non-zero exit stops the deploy.">
          <Textarea label="Build command" rows={3} className="font-mono" value={f.buildCmd} onChange={set("buildCmd")} wrapperClassName="sm:col-span-2" optional />
          <Textarea
            label="Before restart"
            rows={2}
            className="font-mono"
            value={f.beforeRestartCmd}
            onChange={set("beforeRestartCmd")}
            wrapperClassName="sm:col-span-2"
            optional
            helper="Runs after build, before the process restarts (e.g. sqlx migrate run)."
          />
          <TextField label="Start command" mono value={f.startCmd} onChange={set("startCmd")} wrapperClassName="sm:col-span-2" helper={`ExecStart of ${app.unit}, rewritten on save.`} />
          <TextField label="Port" mono inputMode="numeric" value={f.port} onChange={set("port")} />
          <TextField label="Health check" mono value={f.healthPath} onChange={set("healthPath")} optional helper="Must answer 2xx within 30 s, or the deploy rolls back." />
          <TextField
            label="Run as user"
            mono
            value={f.runAs}
            onChange={set("runAs")}
            helper={f.runAs === "root" ? "Full access to the server. Prefer a normal user." : "Saving hands the app folder to this user. Restart to apply."}
          />
        </Group>
        <Group title="Auto-deploy" desc="GitHub calls the webhook on push. Only pushes to the deploy branch start a deploy.">
          <ReadOnly label="Webhook URL" value={webhookUrl(app.webhookPath, publicUrl)} wide />
          <ReadOnly label="Secret" value="••••••••••••••••••••••••" helper="The shared webhook secret lives in Settings." />
          <TextField label="Deploy timeout (min)" mono inputMode="numeric" value={f.timeoutMin} onChange={set("timeoutMin")} helper="Per build / before-restart command." />
        </Group>
        {err && (
          <div className="px-6 pb-4">
            <InlineAlert title="Not saved">{err}</InlineAlert>
          </div>
        )}
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-sunken px-6 py-4">
          <span className="text-13 text-ink-2">{dirty.length ? `${dirty.length} unsaved ${dirty.length === 1 ? "change" : "changes"}.` : "Changes to commands apply on the next deploy."}</span>
          <div className="flex gap-2">
            <Button variant="secondary" disabled={!dirty.length || saving} onClick={() => setF(base)}>
              Discard
            </Button>
            <Button busy={saving} disabled={!dirty.length} onClick={save}>
              {saving ? "Saving…" : "Save changes"}
            </Button>
          </div>
        </div>
      </section>

      <section className="overflow-hidden rounded-xl border border-danger/30 bg-surface">
        <div className="border-b border-line px-6 py-4">
          <h2 className="text-17 font-semibold">Danger zone</h2>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-5">
          <div className="flex max-w-xl flex-col gap-0.5">
            <span className="text-sm font-semibold">{stopped ? "Start app" : "Stop app"}</span>
            <span className="text-13 text-ink-2">
              {stopped
                ? `systemd starts ${app.unit} again with the live release.`
                : `systemd stops the process.${app.domain ? " nginx returns 502 until you start it again." : ""} Files and env stay.`}
            </span>
          </div>
          <Button variant="secondary" busy={stopping} disabled={app.release == null && stopped} onClick={toggleRun}>
            {stopped ? (stopping ? "Starting…" : "Start app") : stopping ? "Stopping…" : "Stop app"}
          </Button>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-4 border-t border-line-soft px-6 py-5">
          <div className="flex max-w-xl flex-col gap-0.5">
            <span className="text-sm font-semibold">Delete app</span>
            <span className="text-13 text-ink-2">Removes the process, deploy history and webhook. You choose whether the folder goes too. Databases and nginx sites are kept.</span>
          </div>
          <Button variant="danger" onClick={() => setDel(true)}>
            Delete app…
          </Button>
        </div>
      </section>

      <ConfirmDialog
        open={del}
        onClose={() => setDel(false)}
        onConfirm={remove}
        busy={deleting}
        title={`Delete ${app.name}?`}
        confirmLabel={deleteFolder ? "Delete app and folder" : "Delete app"}
        confirmText={app.name}
      >
        <p>
          The systemd unit <span className="font-mono">{app.unit}</span> stops and is removed, with <strong>{app.deployCount} deploys</strong> of history and the webhook.
          {app.domain ? ` ${app.domain} stops answering.` : ""} This can&apos;t be undone.
        </p>
        <div className="mt-3 flex flex-col">
          <Checkbox
            checked={deleteFolder}
            onChange={(e) => setDeleteFolder(e.target.checked)}
            label={
              <span className="font-normal">
                Delete folder <span className="font-mono">{app.path}</span> <span className="text-ink-3">(releases, repo, .env)</span>
              </span>
            }
          />
        </div>
      </ConfirmDialog>
    </div>
  );
}
