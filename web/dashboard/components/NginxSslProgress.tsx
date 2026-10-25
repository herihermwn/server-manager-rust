import { useEffect, useRef, useState } from "react";
import { apiUrl } from "../lib/api";
import { Button } from "./Button";
import { LogView, type LogLine } from "./LogView";

type StepState = "wait" | "run" | "done" | "fail";
interface Step {
  label: string;
  hint: string;
  state: StepState;
  meta: string;
}

const STEPS: Omit<Step, "state" | "meta">[] = [
  { label: "DNS check", hint: "A records point to this server" },
  { label: "Get certificate", hint: "certbot · HTTP challenge in /.well-known/acme-challenge" },
  { label: "Test & reload nginx", hint: "nginx -t && systemctl reload nginx" },
];

/**
 * "Issuing SSL certificate" card (NginxSsl.dc.html). Follows GET /api/sites/{id}/ssl/log, which
 * replays what already happened and then stays live. Events: step, stdout, stderr, done.
 */
export function NginxSslProgress({ siteId, onDone, onDismiss }: { siteId: number; onDone: (ok: boolean) => void; onDismiss: () => void }) {
  const [steps, setSteps] = useState<Step[]>(STEPS.map((s) => ({ ...s, state: "wait", meta: "" })));
  const [lines, setLines] = useState<LogLine[]>([]);
  const [result, setResult] = useState<{ ok: boolean; error?: string } | null>(null);
  const [secs, setSecs] = useState(0);
  const doneRef = useRef(false);
  const onDoneRef = useRef(onDone);
  onDoneRef.current = onDone;

  useEffect(() => {
    const es = new EventSource(apiUrl(`/api/sites/${siteId}/ssl/log`));
    const add = (e: MessageEvent) => setLines((p) => [...p.slice(-2000), { text: e.data }]);
    es.addEventListener("stdout", add);
    es.addEventListener("stderr", add);
    es.addEventListener("step", (e) => {
      const { idx, state, meta } = JSON.parse((e as MessageEvent).data) as { idx: number; state: StepState; meta: string };
      setSteps((p) => p.map((s, i) => (i === idx ? { ...s, state, meta: meta || s.meta } : s)));
    });
    es.addEventListener("done", (e) => {
      doneRef.current = true;
      const d = JSON.parse((e as MessageEvent).data) as { code: number | null; error?: string };
      setResult({ ok: d.code === 0, error: d.error });
      es.close();
      onDoneRef.current(d.code === 0);
    });
    es.onerror = () => {
      // The browser would reconnect and the server would replay from line 1: stop instead.
      if (doneRef.current) return;
      es.close();
      doneRef.current = true;
      setResult({ ok: false, error: "Lost the connection to the log. The request may still be running; reload this page to see." });
    };
    return () => es.close();
  }, [siteId]);

  useEffect(() => {
    if (result) return;
    const t = setInterval(() => setSecs((s) => s + 1), 1000);
    return () => clearInterval(t);
  }, [result]);

  return (
    <section role="status" aria-label="Issuing SSL certificate" className="flex flex-wrap gap-6 rounded-xl border border-line bg-surface p-5">
      <div className="flex min-w-[300px] flex-1 flex-col gap-1">
        <div className="mb-2 flex items-baseline justify-between gap-3">
          <h2 className="text-17 font-semibold text-ink">{result ? (result.ok ? "SSL certificate issued" : "SSL certificate not issued") : "Issuing SSL certificate"}</h2>
          <span className="font-mono text-13 font-medium text-ink-2">
            {Math.floor(secs / 60)}:{String(secs % 60).padStart(2, "0")}
          </span>
        </div>
        <ol className="m-0 flex list-none flex-col p-0">
          {steps.map((s) => (
            <li key={s.label} className="flex min-h-10 items-start gap-3">
              <span
                className={`flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-full text-xs font-bold ${
                  s.state === "done" ? "bg-success text-ink-on" : s.state === "run" ? "bg-info-bg text-info" : s.state === "fail" ? "bg-danger text-ink-on" : "border border-dashed border-line-strong"
                }`}
                aria-hidden="true"
              >
                {s.state === "done" ? "✓" : s.state === "fail" ? "!" : s.state === "run" ? "…" : ""}
              </span>
              <div className="flex flex-col">
                <span className={`text-sm ${s.state === "wait" ? "text-ink-3" : "text-ink"} ${s.state === "run" ? "font-semibold" : "font-medium"}`}>
                  {s.label}
                  <span className="sr-only">
                    {" "}
                    {s.state === "done" ? "done" : s.state === "run" ? "running" : s.state === "fail" ? "failed" : "waiting"}
                  </span>
                </span>
                <span className="font-mono text-xs text-ink-3">{s.meta || s.hint}</span>
              </div>
            </li>
          ))}
        </ol>
        {result && !result.ok && result.error && (
          <p role="alert" className="mt-3 rounded-lg bg-danger-bg px-3 py-2.5 text-13 leading-5 text-danger">
            {result.error}
          </p>
        )}
        {result && (
          <div className="mt-3">
            <Button variant="secondary" onClick={onDismiss}>
              Close
            </Button>
          </div>
        )}
      </div>
      <div className="min-w-0 flex-[2_1_420px]">
        <LogView title="certbot" lines={lines} live={!result} height={260} />
      </div>
    </section>
  );
}
