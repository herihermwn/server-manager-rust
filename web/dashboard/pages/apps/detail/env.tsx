import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../../components/Button";
import { EnvImportModal } from "../../../components/EnvImportModal";
import { EnvTable, keyErrors, newRow, rowFromVar, rowStatus, type EnvRow, type EnvSaved, type EnvSnapshot } from "../../../components/EnvTable";
import { Banner, EmptyState, ErrorState, Skeleton } from "../../../components/Feedback";
import { PlusIcon, SearchIcon } from "../../../components/icons";
import { TextField } from "../../../components/Input";
import { useToast } from "../../../components/Toast";
import { ApiError, api } from "../../../lib/api";
import { formatWib } from "../../../lib/server";

/** AppEnv / AppEnvEditing / AppEnvImport. Rendered by pages/apps/detail/index.tsx under the app header. */
export function AppEnvTab({ name }: { name: string }) {
  const toast = useToast();
  const base = `/api/apps/${encodeURIComponent(name)}/env`;
  const [snap, setSnap] = useState<EnvSnapshot | null>(null);
  const [rows, setRows] = useState<EnvRow[]>([]);
  const [loadError, setLoadError] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [notice, setNotice] = useState<{ tone: "warning" | "error"; text: string; reload?: boolean } | null>(null);
  const [filter, setFilter] = useState("");
  const [importing, setImporting] = useState(false);
  const [focusId, setFocusId] = useState<number | null>(null);
  const ids = useRef(1);

  const adopt = useCallback((s: EnvSnapshot) => {
    setSnap(s);
    setRows(s.vars.map((v) => rowFromVar(v, ids.current++)));
  }, []);

  const load = useCallback(() => {
    setLoading(true);
    setLoadError("");
    return api<EnvSnapshot>(base)
      .then(adopt)
      .catch((e: Error) => setLoadError(e.message))
      .finally(() => setLoading(false));
  }, [base, adopt]);

  useEffect(() => void load(), [load]);

  const errors = useMemo(() => keyErrors(rows), [rows]);
  const counts = useMemo(() => {
    const c = { added: 0, changed: 0, removed: 0 };
    for (const r of rows) {
      const s = rowStatus(r);
      if (s === "new") c.added++;
      else if (s === "changed") c.changed++;
      else if (s === "removed") c.removed++;
    }
    return c;
  }, [rows]);
  const dirty = counts.added + counts.changed + counts.removed;
  // A new row you removed again leaves no trace, so only live new rows count.
  const liveNew = rows.filter((r) => r.isNew).length;

  useEffect(() => {
    if (!dirty) return;
    const warn = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);

  const patch = (id: number, p: Partial<EnvRow>) => {
    setNotice(null);
    setRows((rs) => rs.map((r) => (r.id === id ? { ...r, ...p } : r)));
  };

  async function fetchSecret(r: EnvRow): Promise<string | null> {
    if (r.value !== null) return r.value;
    if (r.shown !== null) return r.shown;
    patch(r.id, { busy: true });
    try {
      const { value } = await api<{ value: string }>(`${base}/reveal`, { body: { key: r.key } });
      patch(r.id, { shown: value, busy: false });
      return value;
    } catch (e) {
      patch(r.id, { busy: false });
      toast({ title: `Could not read ${r.key}`, body: (e as Error).message, tone: "error" });
      return null;
    }
  }

  async function reveal(r: EnvRow) {
    if (r.show) return patch(r.id, { show: false });
    if (await fetchSecret(r) !== null) patch(r.id, { show: true });
  }

  async function copy(r: EnvRow) {
    const v = r.secret ? await fetchSecret(r) : r.value;
    if (v === null) return;
    try {
      await navigator.clipboard.writeText(v);
      toast({ title: `Copied ${r.key}`, tone: "info" });
    } catch {
      toast({ title: "Could not copy", body: "The browser blocked clipboard access.", tone: "error" });
    }
  }

  function remove(r: EnvRow) {
    setNotice(null);
    if (r.isNew) setRows((rs) => rs.filter((x) => x.id !== r.id));
    else patch(r.id, { removed: !r.removed });
  }

  function add() {
    const id = ids.current++;
    setFocusId(id);
    setFilter("");
    setNotice(null);
    setRows((rs) => [...rs, newRow(id)]);
  }

  function discard() {
    setNotice(null);
    if (snap) adopt(snap);
  }

  function done(res: EnvSaved) {
    adopt(res.env);
    const { added, changed, removed } = res.changes;
    const parts = [added.length && `${added.length} added`, changed.length && `${changed.length} changed`, removed.length && `${removed.length} removed`].filter(Boolean);
    const rs = res.restart;
    if (rs && !rs.ok && !rs.skipped) setNotice({ tone: "warning", text: `Saved, but ${name} did not restart. ${rs.note}` });
    toast({
      title: parts.length ? "Environment saved" : "Nothing to change",
      body: [parts.join(", "), rs?.ok ? `${name} restarted.` : rs?.skipped ? rs.note : ""].filter(Boolean).join(" "),
      tone: "success",
    });
  }

  async function save() {
    setSaving(true);
    setNotice(null);
    try {
      const vars = rows.filter((r) => !r.removed).map((r) => ({ key: r.key, value: r.value, secret: r.secret }));
      done(await api<EnvSaved>(base, { method: "PUT", body: { vars, rev: snap?.rev, restart: true } }));
    } catch (e) {
      const conflict = e instanceof ApiError && e.status === 409;
      setNotice({ tone: "error", text: (e as Error).message, reload: conflict });
    } finally {
      setSaving(false);
    }
  }

  if (loading && !snap) {
    return (
      <div className="flex flex-col gap-5" role="status" aria-label="Loading environment">
        <Skeleton className="h-10 w-80" />
        <div className="flex flex-col gap-3 rounded-xl border border-line bg-surface p-5">
          {Array.from({ length: 6 }, (_, i) => (
            <Skeleton key={i} className="h-5 w-full" />
          ))}
        </div>
      </div>
    );
  }
  if (loadError || !snap) {
    return <ErrorState title="Could not load the environment" description={loadError || "Try again in a moment."} onRetry={load} busy={loading} />;
  }

  const q = filter.trim().toLowerCase();
  const visible = q ? rows.filter((r) => r.isNew || r.key.toLowerCase().includes(q)) : rows;
  const fix = errors.size;
  const shared = rows.filter((r) => !r.isNew).length;
  const total = shared - counts.removed + liveNew;
  const summary = [counts.added && `${counts.added} added`, counts.changed && `${counts.changed} changed`, counts.removed && `${counts.removed} removed`].filter(Boolean).join(", ");

  return (
    <div className="flex flex-col gap-5">
      {dirty > 0 && (
        <div role="status" className="flex flex-wrap items-center gap-3 rounded-xl bg-info-bg py-3 pl-4 pr-3 text-info-ink">
          <span className="h-2 w-2 shrink-0 rounded-full bg-info" aria-hidden="true" />
          <div className="min-w-[280px] flex-1 text-sm leading-5">
            <strong className="font-semibold">
              {dirty} unsaved {dirty === 1 ? "change" : "changes"}
            </strong>
            {` (${summary})`}
            {fix > 0 ? `, ${fix} to fix before saving` : ""}. Saving writes .env and restarts {name} (2–4 s).
          </div>
          <Button variant="secondary" onClick={discard} disabled={saving}>
            Discard
          </Button>
          <Button onClick={save} busy={saving} disabled={fix > 0}>
            {saving ? "Saving…" : "Save & restart"}
          </Button>
        </div>
      )}
      {notice && <Banner tone={notice.tone} action={notice.reload ? { label: "Reload", onClick: () => void load() } : undefined}>{notice.text}</Banner>}

      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-col gap-0.5">
          <span className="text-sm">
            <span className="font-mono">shared/.env</span> · {total} {total === 1 ? "variable" : "variables"} · {snap.mtime ? `saved ${formatWib(snap.mtime)}` : "not created yet"}
          </span>
          <span className="text-xs text-ink-3">Written to the server with mode 600, owner ubuntu. Secret values stay hidden here and in deploy logs.</span>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative w-60">
            <SearchIcon size={16} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-ink-3" />
            <TextField label="Filter variables" hideLabel placeholder="Filter keys" value={filter} onChange={(e) => setFilter(e.target.value)} className="pl-9" type="search" autoComplete="off" />
          </div>
          <Button variant="secondary" onClick={() => setImporting(true)} disabled={dirty > 0} title={dirty > 0 ? "Save or discard your changes first" : undefined}>
            Paste .env
          </Button>
          <Button icon={<PlusIcon size={16} />} onClick={add}>
            Add variable
          </Button>
        </div>
      </div>

      <EnvTable
        rows={visible}
        errors={errors}
        focusId={focusId}
        onChange={patch}
        onReveal={reveal}
        onCopy={copy}
        onRemove={remove}
        empty={
          q ? (
            <EmptyState title="No variables match" description={`Nothing named like "${filter.trim()}". Clear the filter to see all ${rows.length}.`} />
          ) : (
            <EmptyState title="No variables yet" description="Add one, or paste a .env from your machine. Saving creates shared/.env with mode 600." />
          )
        }
      />

      <EnvImportModal
        open={importing}
        appName={name}
        onClose={() => setImporting(false)}
        onApplied={(res) => {
          setImporting(false);
          done(res);
        }}
      />
    </div>
  );
}
