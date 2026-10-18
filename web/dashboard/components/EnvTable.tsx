import type { ReactNode } from "react";
import { IconButton } from "./Button";
import { CopyIcon, EyeIcon, EyeOffIcon, RefreshIcon, TrashIcon } from "./icons";

/** GET /api/apps/{name}/env */
export interface EnvVar {
  key: string;
  /** null for secrets (the server never sends them). */
  value: string | null;
  secret: boolean;
  /** e.g. "database:radio_api" when the databases page wrote the key. */
  source: string | null;
  empty: boolean;
}
export interface EnvSnapshot {
  path: string;
  exists: boolean;
  mtime: number | null;
  rev: string;
  vars: EnvVar[];
}
/** PUT /env and POST /env/import answer. */
export interface EnvSaved {
  env: EnvSnapshot;
  changes: { added: string[]; changed: string[]; removed: string[] };
  restart: { ok: boolean; skipped: boolean; note: string } | null;
}

/** One table row = one server variable (edited in place) or a new one. */
export interface EnvRow {
  id: number;
  key: string;
  isNew: boolean;
  removed: boolean;
  /** Current value; null = keep what the server has (a secret nobody typed into). */
  value: string | null;
  /** Server value of a plain variable; null for secrets. */
  orig: string | null;
  secret: boolean;
  source: string | null;
  /** Secret value fetched with Reveal (logged on the server). */
  shown: string | null;
  show: boolean;
  busy: boolean;
}

export type RowStatus = "new" | "changed" | "removed" | "same";

const SECRET_HINTS = ["SECRET", "KEY", "TOKEN", "DSN", "PASSWORD", "DATABASE_URL"];
/** Same rule as the server (routes/env.rs `auto_secret`). */
export const autoSecret = (key: string) => SECRET_HINTS.some((h) => key.toUpperCase().includes(h));

export const MASK = "••••••••••••••••";

export function rowFromVar(v: EnvVar, id: number): EnvRow {
  return { id, key: v.key, isNew: false, removed: false, value: v.secret ? null : (v.value ?? ""), orig: v.secret ? null : (v.value ?? ""), secret: v.secret, source: v.source, shown: null, show: false, busy: false };
}

export function newRow(id: number): EnvRow {
  return { id, key: "", isNew: true, removed: false, value: "", orig: null, secret: false, source: null, shown: null, show: false, busy: false };
}

export function rowStatus(r: EnvRow): RowStatus {
  if (r.isNew) return "new";
  if (r.removed) return "removed";
  if (r.value !== null && r.value !== (r.orig ?? r.shown)) return "changed";
  return "same";
}

const KEY_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** Row id → message. Only new rows have editable keys, so only they can be wrong. */
export function keyErrors(rows: EnvRow[]): Map<number, string> {
  const out = new Map<number, string>();
  rows.forEach((r, i) => {
    if (!r.isNew || r.removed) return;
    if (!r.key) return void out.set(r.id, "Enter a key.");
    if (r.key.length > 128 || !KEY_RE.test(r.key)) return void out.set(r.id, "Use letters, digits and _, not starting with a digit.");
    // Compare with every kept row before it, and with saved rows after it: the older one is named.
    const other = rows.find((o, j) => o.id !== r.id && !o.removed && o.key.toLowerCase() === r.key.toLowerCase() && (j < i || !o.isNew));
    if (other) out.set(r.id, `${other.key} already exists. Keys are case sensitive; use one.`);
  });
  return out;
}

const TAG = "inline-flex h-6 items-center whitespace-nowrap rounded-md px-2 text-xs font-semibold";

function TypeTag({ row, status }: { row: EnvRow; status: RowStatus }) {
  if (status === "removed") return <span className={`${TAG} bg-danger-bg text-danger-ink`}>Removed</span>;
  if (status === "new") return <span className={`${TAG} bg-muted text-ink`}>{row.secret ? "New · secret" : "New"}</span>;
  if (status === "changed") return <span className={`${TAG} bg-info-bg text-info-strong`}>Changed</span>;
  if (row.source) return <span className={`${TAG} bg-info-bg text-info-strong`}>{sourceLabel(row.source)}</span>;
  if (row.secret) return <span className={`${TAG} bg-warning-bg text-warning-ink`}>Secret</span>;
  return <span className={`${TAG} font-medium text-ink-3`}>Plain</span>;
}

export function sourceLabel(source: string) {
  return source.startsWith("database:") ? `From database ${source.slice(9)}` : `From ${source}`;
}

interface Props {
  rows: EnvRow[];
  errors: Map<number, string>;
  focusId: number | null;
  onChange: (id: number, patch: Partial<EnvRow>) => void;
  onReveal: (row: EnvRow) => void;
  onCopy: (row: EnvRow) => void;
  onRemove: (row: EnvRow) => void;
  /** Rendered as a full-width row when there is nothing to show. */
  empty?: ReactNode;
}

const TH = "px-3 py-2.5 text-left text-xs font-semibold uppercase tracking-caps text-ink-3";

export function EnvTable({ rows, errors, focusId, onChange, onReveal, onCopy, onRemove, empty }: Props) {
  return (
    <section className="overflow-hidden rounded-xl border border-line bg-surface">
      <div className="overflow-x-auto">
        <table className="w-full min-w-[900px] border-collapse text-sm">
          <thead>
            <tr className="bg-sunken">
              <th scope="col" className={`${TH} w-[280px] px-5`}>Key</th>
              <th scope="col" className={TH}>Value</th>
              <th scope="col" className={`${TH} w-[170px]`}>Type</th>
              <th scope="col" className="w-[140px] px-5 py-2.5">
                <span className="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td colSpan={4} className="border-t border-line-soft">{empty}</td>
              </tr>
            )}
            {rows.map((r) => {
              const status = rowStatus(r);
              const err = errors.get(r.id);
              const tint = err ? "bg-danger-bg/40" : status === "changed" || status === "new" ? "bg-info-bg/50" : status === "removed" ? "bg-danger-bg/40" : "";
              const shownText = r.value ?? (r.show ? r.shown : null);
              const masked = r.secret && !r.show;
              return (
                <tr key={r.id} className={tint}>
                  <td className="h-14 border-t border-line-soft px-5 py-1.5 align-middle">
                    {r.isNew ? (
                      <input
                        aria-label="Key"
                        aria-invalid={!!err}
                        aria-describedby={err ? `env-err-${r.id}` : undefined}
                        value={r.key}
                        autoFocus={r.id === focusId}
                        placeholder="KEY"
                        autoComplete="off"
                        spellCheck={false}
                        onChange={(e) => {
                          const key = e.target.value.trim();
                          onChange(r.id, { key, secret: autoSecret(key) });
                        }}
                        className={`h-10 w-full rounded-lg bg-surface px-3 font-mono text-13 font-medium ${err ? "border-[1.5px] border-danger" : "border border-line-strong"}`}
                      />
                    ) : (
                      <span className={`font-mono text-13 font-medium ${status === "removed" ? "text-ink-3 line-through" : ""}`}>{r.key}</span>
                    )}
                    {err && (
                      <div id={`env-err-${r.id}`} className="pt-1 text-xs text-danger">
                        {err}
                      </div>
                    )}
                  </td>
                  <td className="border-t border-line-soft px-3 py-1.5 align-middle">
                    <input
                      aria-label={`Value of ${r.key || "new variable"}`}
                      type={masked ? "password" : "text"}
                      value={shownText ?? ""}
                      placeholder={r.secret && r.value === null && !r.show ? MASK : r.isNew ? "Value" : ""}
                      disabled={r.removed}
                      autoComplete={masked ? "new-password" : "off"}
                      spellCheck={false}
                      onChange={(e) => onChange(r.id, { value: e.target.value })}
                      className={`h-10 w-full rounded-lg border bg-transparent px-3 font-mono text-13 placeholder:text-ink-3 ${
                        r.isNew ? "border-line-strong bg-surface" : "border-transparent hover:border-line-strong focus:bg-surface"
                      } ${r.removed ? "text-ink-3 line-through" : "text-ink"}`}
                    />
                  </td>
                  <td className="border-t border-line-soft px-3 py-1.5 align-middle">
                    <TypeTag row={r} status={status} />
                  </td>
                  <td className="whitespace-nowrap border-t border-line-soft py-0 pl-3 pr-5 text-right align-middle">
                    {r.secret && !r.removed && (
                      <IconButton
                        variant="ghost"
                        size="sm"
                        label={`${r.show ? "Hide" : "Reveal"} ${r.key}`}
                        aria-pressed={r.show}
                        busy={r.busy}
                        disabled={!r.key}
                        onClick={() => onReveal(r)}
                      >
                        {r.show ? <EyeOffIcon size={16} /> : <EyeIcon size={16} />}
                      </IconButton>
                    )}
                    <IconButton variant="ghost" size="sm" label={`Copy ${r.key}`} disabled={!r.key || r.removed || r.isNew && r.value === ""} onClick={() => onCopy(r)}>
                      <CopyIcon size={16} />
                    </IconButton>
                    {r.removed ? (
                      <IconButton variant="ghost" size="sm" label={`Restore ${r.key}`} onClick={() => onRemove(r)}>
                        <RefreshIcon size={16} />
                      </IconButton>
                    ) : (
                      <IconButton variant="ghost" size="sm" label={`Remove ${r.key || "new variable"}`} onClick={() => onRemove(r)}>
                        <TrashIcon size={16} />
                      </IconButton>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}
