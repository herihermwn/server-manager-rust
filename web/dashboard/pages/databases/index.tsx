import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import { Button } from "../../components/Button";
import { KpiStrip } from "../../components/Card";
import { DataTable, type Column } from "../../components/DataTable";
import { Badge } from "../../components/Badge";
import { ErrorState } from "../../components/Feedback";
import { PageHeader } from "../../components/PageHeader";
import { ConfirmDialog } from "../../components/Modal";
import { useToast } from "../../components/Toast";
import { CopyIcon, DatabaseIcon, PlusIcon, RefreshIcon } from "../../components/icons";
import { DbCreateModal } from "../../components/DbCreateModal";
import { DbCreatedModal } from "../../components/DbCreatedModal";
import { DbRowMenu } from "../../components/DbRowMenu";
import { copyText, fmtSize, when, type DbCreated, type DbItem, type DbStatus } from "../../components/DbApi";

/** pg_dump of every database, 02:00 WIB. Matches the schedule in src/routes/databases.rs. */
const BACKUP_NOTE = "daily 02:00 WIB, keep 7";

function NotReady({ status, onRetry, busy }: { status: DbStatus; onRetry: () => void; busy: boolean }) {
  const toast = useToast();
  const missing = status.state === "missing";
  return (
    <section className="flex flex-col items-center gap-3 rounded-xl border border-line bg-surface px-8 py-14 text-center">
      <div className="flex h-[52px] w-[52px] items-center justify-center rounded-[14px] bg-muted text-ink-2">
        <DatabaseIcon size={24} />
      </div>
      <h2 className="mt-1 text-xl font-semibold">{missing ? "PostgreSQL is not installed" : "Can’t reach PostgreSQL"}</h2>
      <p className="max-w-[520px] text-sm leading-[21px] text-ink-2">
        {missing ? (
          <>
            Install it once to create databases here. It listens on 127.0.0.1 only, so apps on this server can reach it and the internet can’t. Daily backups at 02:00 go to{" "}
            <span className="font-mono">{status.backup.dir}</span>. The manager connects with the <span className="font-mono">PG_ADMIN_URL</span> in{" "}
            <span className="font-mono">~/.server-manager/pg.env</span>.
          </>
        ) : (
          <>
            The connection from <span className="font-mono">PG_ADMIN_URL</span> failed{status.error ? <>: {status.error}</> : null}. Check that the service is running with{" "}
            <span className="font-mono">systemctl status postgresql</span>.
          </>
        )}
      </p>
      {missing && (
        <div className="mt-1 rounded-lg bg-console px-3.5 py-2.5 text-left font-mono text-[12.5px] text-console-text">
          <span className="text-term-cmd">$</span> {status.installCommand}
        </div>
      )}
      <div className="mt-2 flex flex-wrap justify-center gap-2">
        {missing && (
          <Button
            variant="subtle"
            icon={<CopyIcon size={16} />}
            onClick={async () => toast(await copyText(status.installCommand) ? { title: "Command copied", body: "Run it in an SSH session, then press Check again.", tone: "success" } : { title: "Could not copy", tone: "error" })}
          >
            Copy command
          </Button>
        )}
        <Button variant="secondary" icon={<RefreshIcon size={16} />} onClick={onRetry} busy={busy}>
          {busy ? "Checking…" : "Check again"}
        </Button>
      </div>
      {missing && <span className="text-xs text-ink-3">Takes about a minute. Uses about 60 MB of disk.</span>}
    </section>
  );
}

function BackupCell({ d }: { d: DbItem }) {
  const b = d.lastBackup;
  if (!b) return <span className="text-ink-3">Not yet · next 02:00</span>;
  if (!b.ok)
    return (
      <span className="text-danger" title={b.error ?? undefined}>
        Failed {when(b.at)}
        {b.error ? ` · ${b.error.length > 28 ? b.error.slice(0, 28) + "…" : b.error}` : ""}
      </span>
    );
  return (
    <span className="text-ink-2">
      {when(b.at)} · {fmtSize(b.sizeBytes)}
    </span>
  );
}

export function Databases() {
  const toast = useToast();
  const [status, setStatus] = useState<DbStatus | null>(null);
  const [dbs, setDbs] = useState<DbItem[] | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [checking, setChecking] = useState(false);
  const [creating, setCreating] = useState(false);
  const [created, setCreated] = useState<DbCreated | null>(null);
  const [fresh, setFresh] = useState<string | null>(null);
  const [backing, setBacking] = useState<string | null>(null);
  const [dropping, setDropping] = useState<DbItem | null>(null);
  const [dropBusy, setDropBusy] = useState(false);

  const load = useCallback(async () => {
    setChecking(true);
    try {
      const s = await api<DbStatus>("/api/databases/status");
      const list = s.state === "ok" ? await api<DbItem[]>("/api/databases") : [];
      setStatus(s); // both at once, so the strip and the table never disagree
      setDbs(list);
      setError(null);
    } catch (e) {
      setError(e as Error);
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    load();
    const t = setInterval(() => !document.hidden && load(), 15_000);
    return () => clearInterval(t);
  }, [load]);

  async function backUp(d: DbItem) {
    setBacking(d.name);
    try {
      const b = await api<{ file: string; sizeBytes: number }>(`/api/databases/${d.name}/backup`, { method: "POST" });
      toast({ title: `Backup of ${d.name} saved`, body: `${b.file} · ${fmtSize(b.sizeBytes)}`, tone: "success" });
    } catch (e) {
      toast({ title: `Backup of ${d.name} failed`, body: (e as Error).message, tone: "error" });
    } finally {
      setBacking(null);
      load();
    }
  }

  async function copyUrl(d: DbItem) {
    const ok = await copyText(d.url);
    toast(
      ok
        ? { title: "URL copied", body: d.app ? `Without the password. It is in ${d.app}’s .env.` : "Without the password. It was shown once, when the database was created.", tone: "success" }
        : { title: "Could not copy", body: d.url, tone: "error" },
    );
  }

  async function drop() {
    if (!dropping) return;
    setDropBusy(true);
    try {
      const r = await api<{ backup: { file: string }; roleDropped: boolean; warnings: string[] }>(`/api/databases/${dropping.name}`, {
        method: "DELETE",
        body: { confirm: dropping.name },
      });
      toast({ title: `Dropped ${dropping.name}`, body: `Dump saved as ${r.backup.file}${r.warnings[0] ? `. ${r.warnings[0]}` : ""}`, tone: "success" });
      setDropping(null);
    } catch (e) {
      toast({ title: `Could not drop ${dropping.name}`, body: (e as Error).message, tone: "error" });
    } finally {
      setDropBusy(false);
      load();
    }
  }

  const ok = status?.state === "ok";
  const pg = status?.pg;
  const bk = status?.backup;

  const columns: Column<DbItem>[] = [
    {
      key: "name",
      header: "Database",
      render: (d) => (
        <div className="py-1.5">
          <span className="font-mono text-sm font-semibold">{d.name}</span>
          {fresh === d.name && (
            <span className="ml-2 align-middle">
              <Badge tone="success" dot={false}>
                New
              </Badge>
            </span>
          )}
          <div className="text-xs text-ink-3">{d.extensions.length ? d.extensions.join(", ") : "—"}</div>
        </div>
      ),
    },
    { key: "owner", header: "Owner", mono: true, render: (d) => <span className="text-ink-2">{d.owner}</span> },
    { key: "size", header: "Size", align: "right", mono: true, render: (d) => fmtSize(d.sizeBytes) },
    { key: "conns", header: "Conns", align: "right", mono: true, render: (d) => d.connections ?? "—" },
    {
      key: "app",
      header: "Used by",
      render: (d) =>
        d.app ? (
          <a href={`#/apps/${d.app}`} className="font-medium text-info">
            {d.app}
          </a>
        ) : (
          <span className="text-ink-3">Not linked</span>
        ),
    },
    { key: "backup", header: "Last backup", className: "whitespace-nowrap text-13", render: (d) => <BackupCell d={d} /> },
    {
      key: "actions",
      header: <span className="sr-only">Actions</span>,
      align: "right",
      className: "whitespace-nowrap",
      render: (d) => (
        <div className="flex items-center justify-end gap-1">
          <Button variant="subtle" size="sm" onClick={() => copyUrl(d)}>
            Copy URL
          </Button>
          <Button variant="secondary" size="sm" busy={backing === d.name} onClick={() => backUp(d)}>
            {backing === d.name ? "Backing up…" : "Back up"}
          </Button>
          <DbRowMenu name={d.name} onDrop={() => setDropping(d)} />
        </div>
      ),
    },
  ];

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Databases"
        subtitle="PostgreSQL on this server. Each database gets its own user, and can be linked to an app’s .env."
        actions={
          ok && (
            <Button icon={<PlusIcon />} onClick={() => setCreating(true)}>
              Create database
            </Button>
          )
        }
      />

      {error && !status ? (
        <ErrorState title="Could not load databases" description={`${error.message} Nothing was changed.`} onRetry={load} busy={checking} />
      ) : !status ? (
        <>
          <KpiStrip loading items={["PostgreSQL", "Databases", "Connections", "Cache hit", "Last backup"].map((label) => ({ label, value: "", meta: "" }))} />
          <DataTable loading columns={columns} data={[]} label="Databases" />
        </>
      ) : !ok ? (
        <NotReady status={status} onRetry={load} busy={checking} />
      ) : (
        <>
          {error && <ErrorState title="Could not refresh" description={`${error.message} Showing the last result.`} onRetry={load} busy={checking} />}
          <KpiStrip
            items={[
              {
                label: "PostgreSQL",
                value: (
                  <span className="flex items-center gap-2">
                    <span className="h-2 w-2 rounded-full bg-success" style={{ boxShadow: "0 0 0 4px #E5F2EA" }} aria-hidden="true" />
                    Active
                  </span>
                ),
                meta: <span className="font-mono">v{pg?.version} · {pg?.listen}</span>,
              },
              { label: "Databases", value: pg?.databases ?? 0, meta: `on disk ${fmtSize(pg?.sizeBytes)}` },
              { label: "Connections", value: `${pg?.connections ?? 0} / ${pg?.maxConnections ?? 0}`, meta: <span className="font-mono">max_connections {pg?.maxConnections}</span> },
              {
                label: "Cache hit",
                value: pg?.cacheHit == null ? "—" : `${(pg.cacheHit * 100).toFixed(1)}%`,
                meta: <span className="font-mono">shared_buffers {pg?.sharedBuffers}</span>,
              },
              {
                label: "Last backup",
                value: bk?.lastAt ? when(bk.lastAt) : "Not yet",
                meta: bk?.lastAt && bk.total > 0 ? `${bk.ok} of ${bk.total} ok · ${fmtSize(bk.sizeBytes)}` : bk && bk.total > 0 ? "Next run 02:00 WIB" : "No databases yet",
              },
            ]}
          />
          <div className="overflow-hidden rounded-xl border border-line bg-surface">
            <DataTable
              framed={false}
              minWidth={960}
              label="Databases"
              columns={columns}
              data={dbs ?? []}
              rowKey={(d) => d.name}
              emptyMessage="No databases yet. Create one to get a user, a password and a DATABASE_URL."
            />
            <div className="border-t border-line px-6 py-3 text-13 text-ink-2">
              Backups: {BACKUP_NOTE} · <span className="font-mono">{bk?.dir}</span> · {fmtSize(bk?.dirBytes)}
            </div>
          </div>
        </>
      )}

      <DbCreateModal
        open={creating}
        onClose={() => setCreating(false)}
        onCreated={(c) => {
          setCreating(false);
          setCreated(c);
          setFresh(c.name);
          load();
        }}
      />
      <DbCreatedModal created={created} onDone={() => setCreated(null)} />
      <ConfirmDialog
        open={!!dropping}
        onClose={() => setDropping(null)}
        onConfirm={drop}
        busy={dropBusy}
        title={`Drop ${dropping?.name}?`}
        confirmLabel="Drop database"
        cancelLabel="Keep database"
        confirmText={dropping?.name}
      >
        A dump is saved to <span className="font-mono">~/backups/postgres</span> first. Then <strong>{dropping?.name}</strong>
        {dropping && fmtSize(dropping.sizeBytes) !== "—" ? ` (${fmtSize(dropping.sizeBytes)})` : ""} and its user <strong>{dropping?.owner}</strong> are removed.
        {dropping?.app ? ` ${dropping.app}’s DATABASE_URL stays in its .env and will stop working.` : ""}
      </ConfirmDialog>
    </div>
  );
}
