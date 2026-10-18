import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from "react";
import { api, ApiError } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { formatWib } from "../../lib/server";
import { Button } from "../../components/Button";
import { ErrorState, InlineAlert, SkeletonBlock } from "../../components/Feedback";
import { PasswordField, TextField } from "../../components/Input";
import { ConfirmDialog, Modal } from "../../components/Modal";
import { PageHeader } from "../../components/PageHeader";
import { useToast } from "../../components/Toast";
import { copy } from "../../components/appsApi";

interface KeyInfo {
  publicKey: string;
  fingerprint: string;
  type: string;
  createdAt: number | null;
}

interface Alerts {
  diskWarn: number;
  diskStop: number;
  cpuPct: number;
  cpuMinutes: number;
  memWarn: number;
}

interface SettingsData {
  server: { host: string; os: string; timezone: string; agent: string };
  appsDir: string;
  portFrom: number;
  portTo: number;
  alerts: Alerts;
  httpsToken: boolean;
  publicUrl: string;
  webhookUrl: string;
  deployKey: { current: KeyInfo; next: KeyInfo | null; usedBy: string[] };
}

interface RepoCheck {
  app: string;
  repo: string;
  ok: boolean;
  message: string;
}

type Form = { ports: string } & Record<keyof Alerts, string>;

const ALERT_FIELDS: { key: keyof Alerts; label: string; unit: string }[] = [
  { key: "diskWarn", label: "Disk warning", unit: "%" },
  { key: "diskStop", label: "Disk stop deploys", unit: "%" },
  { key: "cpuPct", label: "CPU high level", unit: "%" },
  { key: "cpuMinutes", label: "CPU high for", unit: "min" },
  { key: "memWarn", label: "Memory warning", unit: "%" },
];

const toForm = (d: SettingsData): Form => ({
  ports: `${d.portFrom}–${d.portTo}`,
  diskWarn: String(d.alerts.diskWarn),
  diskStop: String(d.alerts.diskStop),
  cpuPct: String(d.alerts.cpuPct),
  cpuMinutes: String(d.alerts.cpuMinutes),
  memWarn: String(d.alerts.memWarn),
});

const errText = (e: unknown) => (e instanceof ApiError ? e.message : "Could not reach the server.");
const day = (s: number | null) => (s ? formatWib(s, { day: "numeric", month: "short", year: "numeric" }) : "—");

/** Label column + content, like AppSettings. */
function Group({ first, title, desc, children }: { first?: boolean; title: string; desc: ReactNode; children: ReactNode }) {
  return (
    <div className={`grid gap-4 px-6 py-6 lg:grid-cols-[260px_1fr] lg:gap-8 ${first ? "" : "border-t border-line-soft"}`}>
      <div className="flex flex-col gap-1">
        <h2 className="text-15 font-semibold">{title}</h2>
        <p className="text-13 leading-5 text-ink-2">{desc}</p>
      </div>
      <div className="flex min-w-0 flex-col gap-4">{children}</div>
    </div>
  );
}

function UnitField({ id, label, unit, value, onChange }: { id: string; label: string; unit: string; value: string; onChange: (v: string) => void }) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <label htmlFor={id} className="text-13 font-semibold">
        {label}
      </label>
      <div className="flex h-11 items-center overflow-hidden rounded-lg border border-line-strong bg-surface has-[input:focus-visible]:border-info has-[input:focus-visible]:outline has-[input:focus-visible]:outline-2 has-[input:focus-visible]:outline-offset-1 has-[input:focus-visible]:outline-info">
        <input
          id={id}
          inputMode="decimal"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="h-full min-w-0 flex-1 bg-transparent px-3 font-mono text-sm focus-visible:outline-none"
        />
        <span className="flex h-full items-center border-l border-line bg-sunken px-2.5 text-13 text-ink-2">{unit}</span>
      </div>
    </div>
  );
}

function KeyBox({ text }: { text: string }) {
  return <div className="break-all rounded-lg border border-line bg-sunken px-3.5 py-3 font-mono text-xs leading-[18px] text-ink">{text}</div>;
}

/** SettingsKey.dc.html: new key shown once + per-repo access with the NEW key alone. */
function NewKeyModal({ open, onClose, keyInfo }: { open: boolean; onClose: () => void; keyInfo: KeyInfo | null }) {
  const toast = useToast();
  const [checks, setChecks] = useState<RepoCheck[] | null>(null);
  const [checking, setChecking] = useState(false);
  const [err, setErr] = useState("");
  const run = useCallback(async () => {
    setChecking(true);
    setErr("");
    try {
      setChecks((await api<{ repos: RepoCheck[] }>("/api/settings/deploy-key/check", { method: "POST" })).repos);
    } catch (e) {
      setErr(errText(e));
    } finally {
      setChecking(false);
    }
  }, []);
  useEffect(() => {
    if (open) run();
  }, [open, run]);

  return (
    <Modal
      open={open}
      onClose={onClose}
      size="lg"
      title="New deploy key created"
      description="The old key keeps working until you remove it, so nothing breaks now. Add the new key to each repository, then remove the old one."
      footer={
        <>
          <Button variant="subtle" disabled={!keyInfo} onClick={() => keyInfo && copy(keyInfo.publicKey)?.then(() => toast({ title: "New key copied", body: "GitHub › repository › Settings › Deploy keys, read-only." }))}>
            Copy public key
          </Button>
          <Button variant="secondary" busy={checking} onClick={run}>
            {checking ? "Checking…" : "Check access again"}
          </Button>
          <Button onClick={onClose}>Done</Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        {keyInfo && <KeyBox text={keyInfo.publicKey} />}
        {err && <InlineAlert title="Could not check access">{err}</InlineAlert>}
        {checks && checks.length === 0 && <p className="text-13 text-ink-2">No app uses an SSH repository yet.</p>}
        {checks && checks.length > 0 && (
          <ul className="flex flex-col" aria-label="Access with the new key">
            {checks.map((c, i) => (
              <li key={c.app} className={`flex min-h-12 items-center gap-3 py-2 ${i ? "border-t border-line-soft" : ""}`}>
                <span className={`h-2 w-2 flex-none rounded-full ${c.ok ? "bg-success" : "bg-warning"}`} aria-hidden="true" />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate font-mono text-13">{c.repo}</span>
                  {!c.ok && <span className="text-xs text-ink-3">{c.message}</span>}
                </span>
                <span className={`text-13 font-semibold ${c.ok ? "text-success" : "text-warning"}`}>{c.ok ? "New key works" : "Not added yet"}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Modal>
  );
}

/** Settings.dc.html (default / newkey). */
export function Settings() {
  const toast = useToast();
  const { user } = useAuth();
  const [data, setData] = useState<SettingsData | null>(null);
  const [loadErr, setLoadErr] = useState("");
  const [f, setF] = useState<Form | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveErr, setSaveErr] = useState("");
  const [busy, setBusy] = useState("");
  const [keyOpen, setKeyOpen] = useState(false);
  const [removeOld, setRemoveOld] = useState(false);
  const [rotateSecret, setRotateSecret] = useState(false);
  const [secret, setSecret] = useState<string | null>(null);
  const [token, setToken] = useState("");
  const [pw, setPw] = useState({ cur: "", next: "", again: "" });
  const [pwErr, setPwErr] = useState("");

  const load = useCallback(async () => {
    try {
      const d = await api<SettingsData>("/api/settings");
      setData(d);
      setF(toForm(d));
      setLoadErr("");
    } catch (e) {
      setLoadErr(errText(e));
    }
  }, []);
  useEffect(() => {
    load();
  }, [load]);

  if (!data || !f) {
    return (
      <div className="flex flex-col gap-6">
        <PageHeader title="Settings" subtitle="Server defaults, git access, alerts and your account." />
        {loadErr ? <ErrorState title="Could not load settings" description={loadErr} onRetry={load} /> : <SkeletonBlock label="Loading settings" />}
      </div>
    );
  }

  const base = toForm(data);
  const dirty = (Object.keys(f) as (keyof Form)[]).filter((k) => f[k] !== base[k]).length;
  const set = (k: keyof Form) => (v: string) => setF({ ...f, [k]: v });
  const key = data.deployKey;

  /** Runs a settings action with a busy flag; errors become a toast (nothing changed). */
  const act = async (name: string, fn: () => Promise<void>) => {
    setBusy(name);
    try {
      await fn();
    } catch (e) {
      toast({ title: "Not done", body: errText(e), tone: "error" });
    } finally {
      setBusy("");
    }
  };

  async function save() {
    if (!f) return;
    setSaveErr("");
    const m = f.ports.trim().match(/^(\d+)\s*[-–]\s*(\d+)$/);
    if (!m) return setSaveErr("Port range: two numbers like 3010–3099.");
    const num = (k: keyof Alerts) => Number(f[k].trim().replace(",", "."));
    const alerts = Object.fromEntries(ALERT_FIELDS.map((a) => [a.key, num(a.key)])) as unknown as Alerts;
    if (Object.values(alerts).some((v) => !Number.isFinite(v))) return setSaveErr("Alert levels must be numbers.");
    setSaving(true);
    try {
      await api("/api/settings", { method: "PUT", body: { portFrom: Number(m[1]), portTo: Number(m[2]), alerts } });
      toast({ title: "Settings saved", body: "New apps and alerts use them from now on.", tone: "success" });
      await load();
    } catch (e) {
      setSaveErr(errText(e));
    } finally {
      setSaving(false);
    }
  }

  async function changePassword(e: FormEvent) {
    e.preventDefault();
    setPwErr("");
    if (pw.next !== pw.again) return setPwErr("The new passwords don’t match.");
    await act("password", async () => {
      try {
        await api("/api/auth/password", { body: { currentPassword: pw.cur, newPassword: pw.next } });
      } catch (err) {
        setPwErr(errText(err));
        return;
      }
      setPw({ cur: "", next: "", again: "" });
      toast({ title: "Password changed", body: "Other sessions are signed out.", tone: "success" });
    });
  }

  return (
    <div className="flex flex-col gap-6">
      <PageHeader title="Settings" subtitle="Server defaults, git access, alerts and your account." />

      <section className="overflow-hidden rounded-xl border border-line bg-surface">
        <Group first title="Server" desc="Defaults for new apps. Changing them doesn’t move existing apps.">
          <dl className="grid gap-3 [grid-template-columns:repeat(auto-fit,minmax(180px,1fr))]">
            {[
              ["Host", data.server.host],
              ["OS", data.server.os],
              ["Timezone", `${data.server.timezone} · UI in WIB`],
              ["Agent", data.server.agent],
            ].map(([k, v]) => (
              <div key={k} className="flex min-w-0 flex-col gap-0.5">
                <dt className="text-xs text-ink-3">{k}</dt>
                <dd className="break-words font-mono text-sm font-medium">{v}</dd>
              </div>
            ))}
          </dl>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="flex min-w-0 flex-col gap-1.5">
              <span className="text-13 font-semibold">Apps folder</span>
              <div className="flex h-11 items-center truncate rounded-lg border border-line bg-paper px-3 font-mono text-sm text-ink-2">{data.appsDir}</div>
              <span className="text-xs text-ink-3">Set by SM_APPS_DIR in the service: every path check is anchored to it.</span>
            </div>
            <TextField label="Port range for new apps" mono value={f.ports} onChange={(e) => set("ports")(e.target.value)} helper="The first free port is suggested." />
          </div>
        </Group>

        <Group title="Git access" desc="One SSH deploy key for every repo on this server. Add it read-only to each repository.">
          <div className="flex flex-col gap-2.5 rounded-[10px] border border-line bg-sunken p-4">
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <span className="text-sm font-semibold">
                {key.next ? "Old deploy key" : "Deploy key"} · {key.current.type}
              </span>
              <span className="text-xs text-ink-2">
                created {day(key.current.createdAt)} · used by {key.usedBy.length} {key.usedBy.length === 1 ? "app" : "apps"}
              </span>
            </div>
            <div className="break-all font-mono text-xs leading-[18px]">{key.current.publicKey}</div>
            <div className="font-mono text-xs text-ink-3">{key.current.fingerprint}</div>
            {key.usedBy.length > 0 && <div className="text-xs text-ink-2">Apps: <span className="font-mono">{key.usedBy.join(", ")}</span></div>}
            <div className="flex flex-wrap gap-2">
              <Button variant="subtle" onClick={() => copy(key.current.publicKey)?.then(() => toast({ title: "Deploy key copied", body: "Paste it in GitHub › Settings › Deploy keys, read-only." }))}>
                Copy public key
              </Button>
              {!key.next && (
                <Button
                  variant="secondary"
                  busy={busy === "key"}
                  onClick={() =>
                    act("key", async () => {
                      await api("/api/settings/deploy-key/new", { method: "POST" });
                      await load();
                      setKeyOpen(true);
                    })
                  }
                >
                  {busy === "key" ? "Generating…" : "Generate new key"}
                </Button>
              )}
            </div>
          </div>
          {key.next && (
            <div className="flex flex-col gap-2.5 rounded-[10px] border border-info/40 bg-info-bg/40 p-4">
              <div className="flex flex-wrap items-baseline justify-between gap-2">
                <span className="text-sm font-semibold">New deploy key · {key.next.type}</span>
                <span className="text-xs text-ink-2">created {day(key.next.createdAt)} · both keys work until you remove the old one</span>
              </div>
              <div className="break-all font-mono text-xs leading-[18px]">{key.next.publicKey}</div>
              <div className="font-mono text-xs text-ink-3">{key.next.fingerprint}</div>
              <div className="flex flex-wrap gap-2">
                <Button variant="subtle" onClick={() => setKeyOpen(true)}>
                  Check access
                </Button>
                <Button variant="danger" onClick={() => setRemoveOld(true)}>
                  Remove old key
                </Button>
              </div>
            </div>
          )}
          <div className="flex flex-col gap-1.5">
            <div className="flex flex-wrap items-end gap-2">
              <PasswordField
                label={
                  <>
                    HTTPS token <span className="font-normal text-ink-3">optional, for https://github.com repo links</span>
                  </>
                }
                wrapperClassName="flex-[1_1_260px]"
                className="font-mono"
                autoComplete="off"
                placeholder={data.httpsToken ? "Set · paste a new token to replace it" : "Not set"}
                value={token}
                onChange={(e) => setToken(e.target.value)}
              />
              <Button
                variant="secondary"
                disabled={!token.trim()}
                busy={busy === "token"}
                onClick={() =>
                  act("token", async () => {
                    await api("/api/settings/https-token", { method: "PUT", body: { token } });
                    setToken("");
                    toast({ title: "HTTPS token saved", tone: "success" });
                    await load();
                  })
                }
              >
                {busy === "token" ? "Saving…" : "Set token"}
              </Button>
              {data.httpsToken && (
                <Button
                  variant="danger"
                  busy={busy === "token-rm"}
                  onClick={() =>
                    act("token-rm", async () => {
                      await api("/api/settings/https-token", { method: "DELETE" });
                      toast({ title: "HTTPS token removed" });
                      await load();
                    })
                  }
                >
                  Remove token
                </Button>
              )}
            </div>
            <span className="text-xs text-ink-3">Fine-grained GitHub token with Contents: read. Kept in a root-only file (mode 600) in the data folder, never shown again and never in app .env files.</span>
          </div>
        </Group>

        <Group title="Auto-deploy webhook" desc="GitHub signs each push with this secret. Requests with a wrong signature are dropped.">
          <div className="flex min-w-0 flex-col gap-1.5">
            <span className="text-13 font-semibold">Payload URL</span>
            <div className="flex h-11 items-center truncate rounded-lg border border-line bg-paper px-3 font-mono text-sm text-ink-2">{data.webhookUrl}</div>
            <span className="text-xs text-ink-3">Replace &lt;app&gt; with the app name. Content type: application/json.</span>
          </div>
          <div className="flex flex-wrap items-end gap-2">
            <div className="flex min-w-0 flex-[1_1_260px] flex-col gap-1.5">
              <span className="text-13 font-semibold">Secret</span>
              <div className="flex h-11 items-center truncate rounded-lg border border-line bg-paper px-3 font-mono text-sm text-ink-2">{secret ?? "••••••••••••••••••••••••"}</div>
            </div>
            {secret ? (
              <Button variant="subtle" onClick={() => copy(secret)?.then(() => toast({ title: "Secret copied" }))}>
                Copy secret
              </Button>
            ) : (
              <Button
                variant="subtle"
                busy={busy === "reveal"}
                onClick={() => act("reveal", async () => setSecret((await api<{ secret: string }>("/api/settings/webhook-secret/reveal", { method: "POST" })).secret))}
              >
                Reveal
              </Button>
            )}
            <Button variant="secondary" onClick={() => setRotateSecret(true)}>
              Regenerate secret
            </Button>
          </div>
        </Group>

        <Group title="Usage alerts" desc="Shown as a banner on Overview and Server usage. Deploys pause above the disk stop level.">
          <div className="grid gap-4 [grid-template-columns:repeat(auto-fit,minmax(150px,1fr))]">
            {ALERT_FIELDS.map((a) => (
              <UnitField key={a.key} id={`al-${a.key}`} label={a.label} unit={a.unit} value={f[a.key]} onChange={set(a.key)} />
            ))}
          </div>
        </Group>

        {saveErr && (
          <div className="px-6 pb-4">
            <InlineAlert title="Not saved">{saveErr}</InlineAlert>
          </div>
        )}
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-sunken px-6 py-4">
          <span className="text-13 text-ink-2">{dirty ? `${dirty} unsaved ${dirty === 1 ? "change" : "changes"}.` : "Port range and alert levels."}</span>
          <div className="flex gap-2">
            <Button variant="secondary" disabled={!dirty || saving} onClick={() => setF(base)}>
              Discard
            </Button>
            <Button busy={saving} disabled={!dirty} onClick={save}>
              {saving ? "Saving…" : "Save settings"}
            </Button>
          </div>
        </div>
      </section>

      <section className="overflow-hidden rounded-xl border border-line bg-surface">
        <div className="grid gap-4 px-6 py-6 lg:grid-cols-[260px_1fr] lg:gap-8">
          <div className="flex flex-col gap-1">
            <h2 className="text-15 font-semibold">Password</h2>
            <p className="text-13 leading-5 text-ink-2">
              Signed in as <span className="font-mono">{user?.username}</span>. Changing it signs out other sessions.
            </p>
          </div>
          <form onSubmit={changePassword} className="flex min-w-0 flex-col gap-4">
            <div className="grid items-start gap-4 [grid-template-columns:repeat(auto-fit,minmax(200px,1fr))]">
              <PasswordField label="Current" autoComplete="current-password" value={pw.cur} onChange={(e) => setPw({ ...pw, cur: e.target.value })} />
              <PasswordField label="New" autoComplete="new-password" value={pw.next} onChange={(e) => setPw({ ...pw, next: e.target.value })} helper="At least 10 characters." />
              <PasswordField label="Repeat new" autoComplete="new-password" value={pw.again} onChange={(e) => setPw({ ...pw, again: e.target.value })} />
            </div>
            {pwErr && <InlineAlert title="Password not changed">{pwErr}</InlineAlert>}
            <div>
              <Button type="submit" busy={busy === "password"} disabled={!pw.cur || !pw.next || !pw.again}>
                {busy === "password" ? "Changing…" : "Change password"}
              </Button>
            </div>
          </form>
        </div>
      </section>

      <NewKeyModal open={keyOpen} onClose={() => setKeyOpen(false)} keyInfo={key.next} />

      <ConfirmDialog
        open={removeOld}
        onClose={() => setRemoveOld(false)}
        busy={busy === "remove-old"}
        title="Remove the old deploy key?"
        confirmLabel="Remove old key"
        onConfirm={() =>
          act("remove-old", async () => {
            await api("/api/settings/deploy-key/remove-old", { method: "POST" });
            setRemoveOld(false);
            toast({ title: "Old key removed", body: "The new key is now the only deploy key.", tone: "success" });
            await load();
          })
        }
      >
        The server stops offering <strong className="break-all font-mono">{key.current.fingerprint}</strong>. {key.usedBy.length} {key.usedBy.length === 1 ? "app" : "apps"} clone over SSH: any repository that doesn’t have the new key yet fails its next deploy. Run “Check access” first.
      </ConfirmDialog>

      <ConfirmDialog
        open={rotateSecret}
        onClose={() => setRotateSecret(false)}
        busy={busy === "secret"}
        tone="primary"
        title="Regenerate the webhook secret?"
        confirmLabel="Regenerate secret"
        onConfirm={() =>
          act("secret", async () => {
            const r = await api<{ secret: string }>("/api/settings/webhook-secret/rotate", { method: "POST" });
            setSecret(r.secret);
            setRotateSecret(false);
            toast({ title: "New webhook secret", body: "Paste it into every GitHub webhook that uses the shared secret.", tone: "success" });
          })
        }
      >
        Pushes signed with the old secret are dropped from now on, until each GitHub webhook has the new one. Apps with their own secret are not affected.
      </ConfirmDialog>
    </div>
  );
}
