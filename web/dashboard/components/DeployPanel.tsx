import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useEventStream } from "../lib/useEventStream";
import { Button } from "./Button";
import { LogView } from "./LogView";
import { CheckIcon, CloseIcon } from "./icons";
import { useToast } from "./Toast";
import { appUrl, fmtDuration, useApi, type DeployDetail, type DeployStep } from "./appsApi";

export type StepState = "done" | "fail" | "run" | "wait" | "skip";

export interface StepItem {
  label: string;
  cmd?: string;
  time?: string;
  state: StepState;
}

const DOT: Record<StepState, string> = {
  done: "bg-success text-white",
  fail: "bg-danger text-white",
  run: "bg-info-bg text-info",
  wait: "border-[1.5px] border-dashed border-line-strong",
  skip: "border-[1.5px] border-line-strong",
};

/** Pipeline step list (AppDetail.dc.html "Deploy #43"): dot + label + mono command + time. */
export function PipelineSteps({ steps }: { steps: StepItem[] }) {
  return (
    <ol className="m-0 flex list-none flex-col p-0">
      {steps.map((s, i) => (
        <li key={i} className="flex min-h-11 items-start gap-3">
          <span className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-full ${DOT[s.state]}`} aria-hidden="true">
            {s.state === "done" && <CheckIcon size={13} strokeWidth={3} />}
            {s.state === "fail" && <CloseIcon size={12} strokeWidth={3} />}
            {s.state === "run" && (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" className="animate-spin">
                <path d="M21 12a9 9 0 1 1-6.2-8.6" />
              </svg>
            )}
          </span>
          <div className="flex min-w-0 flex-1 items-start justify-between gap-3">
            <div className="flex min-w-0 flex-col">
              <span className={`text-sm ${s.state === "run" || s.state === "fail" ? "font-semibold" : "font-medium"} ${s.state === "wait" || s.state === "skip" ? "text-ink-3" : s.state === "fail" ? "text-danger" : "text-ink"}`}>
                {s.label}
                <span className="sr-only">: {{ done: "done", fail: "failed", run: "running", wait: "waiting", skip: "skipped" }[s.state]}</span>
              </span>
              {s.cmd && <span className="break-all font-mono text-xs text-ink-3">{s.cmd}</span>}
            </div>
            {s.time && <span className="shrink-0 font-mono text-xs font-medium text-ink-2">{s.time}</span>}
          </div>
        </li>
      ))}
    </ol>
  );
}

function stepItem(s: DeployStep, now: number): StepItem {
  const state: StepState =
    s.status === "success" ? "done" : s.status === "failed" || s.status === "cancelled" ? "fail" : s.status === "running" ? "run" : s.status === "skipped" ? "skip" : "wait";
  const cmd = s.status === "skipped" ? "skipped, no command" : s.exitCode && s.exitCode !== 0 ? `${s.command} · exit ${s.exitCode}` : s.command;
  const time = s.startedAt ? fmtDuration(Math.round((s.finishedAt ?? now) - s.startedAt)) : "";
  return { label: s.name, cmd, time, state };
}

export const isFinished = (status: string | undefined) => !!status && status !== "queued" && status !== "running";

/**
 * One deploy, live: steps (polled every 2 s while running), log (SSE), Cancel while allowed.
 * `onFinish` fires once when it ends (the parent reloads the app).
 */
export function DeployPanel({ app, deployId, onFinish }: { app: string; deployId: number; onFinish?: (d: DeployDetail) => void }) {
  const toast = useToast();
  const [live, setLive] = useState(true);
  const { data, reload } = useApi<{ data: DeployDetail }>(`${appUrl(app)}/deploys/${deployId}`, live ? 2000 : 0);
  const d = data?.data;
  const { lines, done } = useEventStream(`${appUrl(app)}/deploys/${deployId}/log`);
  const [cancelling, setCancelling] = useState(false);
  const [now, setNow] = useState(Date.now() / 1000);

  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => clearInterval(t);
  }, [live]);

  useEffect(() => {
    if (d && isFinished(d.status) && live) {
      setLive(false);
      onFinish?.(d);
    }
  }, [d?.status]);

  useEffect(() => {
    if (done) reload();
  }, [done]);

  const cancel = async () => {
    setCancelling(true);
    try {
      await api(`${appUrl(app)}/deploys/${deployId}/cancel`, { method: "POST" });
      toast({ title: `Cancelling deploy #${d?.number ?? ""}`, body: "The running step is stopped. The live release is not touched." });
      reload();
    } catch (e) {
      toast({ title: "Could not cancel", body: (e as Error).message, tone: "error" });
    } finally {
      setCancelling(false);
    }
  };

  const failed = d?.status === "failed" || d?.status === "rolled_back";
  const elapsed = d?.startedAt ? fmtDuration(Math.round((d.finishedAt ?? now) - d.startedAt)) : "queued";
  const head =
    !d || !isFinished(d.status) ? elapsed : d.status === "success" ? `done in ${elapsed}` : `${d.status.replace("_", " ")} after ${elapsed}`;

  return (
    <div className="flex flex-wrap items-stretch gap-6">
      <section aria-label={`Deploy ${d ? `#${d.number}` : ""}`} className="flex min-w-0 flex-[1_1_300px] flex-col gap-1 rounded-xl border border-line bg-surface p-5">
        <div className="mb-2 flex items-baseline justify-between gap-3">
          <h2 className="text-17 font-semibold">{d ? `${d.trigger === "rollback" ? "Rollback" : "Deploy"} #${d.number}` : "Deploy"}</h2>
          <span className="font-mono text-13 font-medium text-ink-2">{head}</span>
        </div>
        {d?.commit && (
          <div className="mb-2 text-13 text-ink-2">
            <span className="font-mono">{d.commit}</span> {d.message}
            {d.actor ? ` · by ${d.actor}` : ""}
          </div>
        )}
        <PipelineSteps steps={(d?.steps ?? []).map((s) => stepItem(s, now))} />
        {d?.error && isFinished(d.status) && <p className={`mt-2 text-13 ${failed ? "text-danger" : "text-ink-2"}`}>{d.error}</p>}
        {d?.cancellable && (
          <Button variant="secondary" onClick={cancel} busy={cancelling} className="mt-2 self-start">
            {cancelling ? "Cancelling…" : "Cancel deploy"}
          </Button>
        )}
      </section>
      <div id="deploy-log" className="min-w-0 flex-[2_1_520px]">
        <LogView
          title={`${isFinished(d?.status) ? "Log" : "Live log"} · deploy #${d?.number ?? ""}${failed ? ` (${d?.status.replace("_", " ")})` : ""}`}
          lines={lines.map((l) => ({ text: l.text }))}
          live={!isFinished(d?.status)}
          height={440}
        />
      </div>
    </div>
  );
}
