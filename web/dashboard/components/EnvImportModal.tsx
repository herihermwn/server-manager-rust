import { useEffect, useState } from "react";
import { Button } from "./Button";
import { Checkbox, Textarea } from "./Input";
import { Modal } from "./Modal";
import { Spinner } from "./icons";
import { api } from "../lib/api";
import type { EnvSaved } from "./EnvTable";

interface PreviewItem {
  key: string;
  kind: "new" | "changed" | "same";
  secret: boolean;
  old: string | null;
  new: string | null;
}
interface Preview {
  items: PreviewItem[];
  errors: { line: number; message: string }[];
  new: number;
  changed: number;
  same: number;
}

const KIND = {
  new: "bg-success-bg text-success",
  changed: "bg-info-bg text-info-strong",
  same: "bg-muted text-ink-2",
};

function valueText(i: PreviewItem) {
  if (i.secret) return "•••••••• (secret)";
  if (i.kind === "changed") return `${i.old} → ${i.new}`;
  return i.new ?? "";
}

/** AppEnvImport.dc.html: paste text, live diff against the saved .env, apply (merge, nothing is removed). */
export function EnvImportModal({ open, appName, onClose, onApplied }: { open: boolean; appName: string; onClose: () => void; onApplied: (r: EnvSaved) => void }) {
  const [text, setText] = useState("");
  const [mark, setMark] = useState(true);
  const [restart, setRestart] = useState(true);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [loading, setLoading] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState("");
  const base = `/api/apps/${encodeURIComponent(appName)}/env`;

  useEffect(() => {
    if (open) {
      setText("");
      setPreview(null);
      setError("");
    }
  }, [open]);

  useEffect(() => {
    if (!open || !text.trim()) {
      setPreview(null);
      setLoading(false);
      return;
    }
    const ctl = new AbortController();
    setLoading(true);
    const t = setTimeout(() => {
      api<Preview>(`${base}/preview`, { method: "POST", body: { text, mark_secrets: mark }, signal: ctl.signal })
        .then((p) => {
          setPreview(p);
          setError("");
          setLoading(false);
        })
        .catch((e) => {
          if (ctl.signal.aborted) return;
          setPreview(null);
          setError(e.message);
          setLoading(false);
        });
    }, 250);
    return () => {
      clearTimeout(t);
      ctl.abort();
    };
  }, [open, text, mark, base]);

  const n = preview ? preview.new + preview.changed : 0;
  const blocked = !preview || preview.errors.length > 0 || n === 0 || loading;

  async function apply() {
    setApplying(true);
    try {
      onApplied(await api<EnvSaved>(`${base}/import`, { method: "POST", body: { text, mark_secrets: mark, restart } }));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setApplying(false);
    }
  }

  return (
    <Modal
      open={open}
      onClose={onClose}
      size="lg"
      title="Paste .env"
      description="KEY=value per line. Comments and blank lines are skipped. Keys that are not pasted stay as they are."
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={apply} disabled={blocked} busy={applying}>
            {applying ? "Applying…" : `Apply ${n} ${n === 1 ? "change" : "changes"}`}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        <Textarea label=".env content" hideLabel rows={7} value={text} onChange={(e) => setText(e.target.value)} placeholder={"# from local machine\nRUST_LOG=info\nPORT=3010"} className="whitespace-pre font-mono text-13 leading-5" spellCheck={false} autoComplete="off" />
        {error && (
          <div role="alert" className="rounded-lg bg-danger-bg px-3 py-2.5 text-13 text-danger-ink">
            {error}
          </div>
        )}
        {preview && preview.errors.length > 0 && (
          <ul role="alert" className="flex flex-col gap-1 rounded-lg bg-danger-bg px-3 py-2.5 text-13 text-danger-ink">
            {preview.errors.map((e, i) => (
              <li key={i}>
                <span className="font-mono">Line {e.line}</span> {e.message}
              </li>
            ))}
          </ul>
        )}
        <div className="overflow-hidden rounded-[10px] border border-line" aria-live="polite">
          <div className="flex items-center gap-2 border-b border-line bg-sunken px-3 py-2 text-xs font-semibold uppercase tracking-caps text-ink-3">
            {preview ? `Preview · ${preview.new} new, ${preview.changed} changed, ${preview.same} same` : "Preview"}
            {loading && <Spinner size={12} />}
          </div>
          {preview && preview.items.length > 0 ? (
            <div className="max-h-64 overflow-y-auto">
              {preview.items.map((i, idx) => (
                <div key={i.key} className={`flex min-h-10 items-center gap-3 px-3 font-mono text-13 ${idx ? "border-t border-line-soft" : ""}`}>
                  <span className={`inline-flex h-[22px] w-[72px] shrink-0 items-center justify-center rounded-md font-sans text-[11px] font-semibold uppercase tracking-[0.04em] ${KIND[i.kind]}`}>{i.kind}</span>
                  <span className="shrink-0 font-medium">{i.key}</span>
                  <span className="min-w-0 flex-1 truncate text-ink-2">{valueText(i)}</span>
                </div>
              ))}
            </div>
          ) : (
            <div className="px-3 py-4 text-13 text-ink-3">{text.trim() ? (loading ? "Reading…" : "No variables found in this text.") : "Paste some KEY=value lines to see what would change."}</div>
          )}
        </div>
        <div className="flex flex-col">
          <Checkbox checked={mark} onChange={(e) => setMark(e.target.checked)} label="Mark new keys with SECRET, KEY, TOKEN, DSN or PASSWORD as secret" />
          <Checkbox checked={restart} onChange={(e) => setRestart(e.target.checked)} label={`Restart ${appName} after applying`} />
        </div>
      </div>
    </Modal>
  );
}
