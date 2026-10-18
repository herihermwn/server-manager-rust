import { useState } from "react";
import { api } from "../../../lib/api";
import { usePageCrumb } from "../../../components/Layout";
import { Badge, Tag, type Tone } from "../../../components/Badge";
import { Button } from "../../../components/Button";
import { ErrorState, SkeletonBlock } from "../../../components/Feedback";
import { useToast } from "../../../components/Toast";
import { BranchIcon, DeployIcon, ExternalIcon, RestartIcon } from "../../../components/icons";
import { appUrl, runtimeLabel, useApi, type AppDetailData } from "../../../components/appsApi";
import { AppOverviewTab } from "./overview";
import { AppEnvTab } from "./env";
import { AppSettingsTab } from "./settings";

export type AppTab = "overview" | "env" | "settings";

const TABS: { key: AppTab; label: string; suffix: string }[] = [
  { key: "overview", label: "Overview & deploys", suffix: "" },
  { key: "env", label: "Environment", suffix: "/env" },
  { key: "settings", label: "Settings", suffix: "/settings" },
];

export const lastFailed = (a: AppDetailData) => a.lastDeploy?.status === "failed" || a.lastDeploy?.status === "rolled_back";

function headerBadge(a: AppDetailData): [string, Tone] {
  if (a.status === "deploying") return [`Deploying #${a.activeDeploy?.number ?? ""}`, "info"];
  if (a.status === "crashed") return ["Crashed", "danger"];
  if (a.status === "running") return lastFailed(a) ? ["Running · last deploy failed", "warning"] : ["Running", "success"];
  return ["Stopped", "neutral"];
}

const shortRepo = (r: string) => r.replace(/^(git@[^:]+:|https?:\/\/[^/]+\/)/, "").replace(/\.git$/, "");

/**
 * AppHeader.dc.html: name, status, runtime · systemd, repo/path/port line, Restart + Deploy, tabs.
 * Phase 3 owns this shell and the overview/settings tabs; phase 4 owns env.tsx.
 * Routes: #/apps/<name>, #/apps/<name>/env, #/apps/<name>/settings.
 */
export function AppDetail({ name, tab }: { name: string; tab: AppTab }) {
  usePageCrumb(name);
  const toast = useToast();
  const { data, error, reload } = useApi<{ data: AppDetailData }>(appUrl(name), 4000);
  const app = data?.data;
  const deploying = app?.status === "deploying";
  const [busy, setBusy] = useState<"restart" | "deploy" | null>(null);

  const run = async (what: "restart" | "deploy") => {
    setBusy(what);
    try {
      await api(what === "deploy" ? `${appUrl(name)}/deploys` : `${appUrl(name)}/restart`, { method: "POST" });
      toast({ title: what === "deploy" ? `Deploy of ${name} started` : `${name} restarted`, tone: what === "deploy" ? "info" : "success" });
      reload();
    } catch (e) {
      toast({ title: what === "deploy" ? "Deploy not started" : "Restart failed", body: (e as Error).message, tone: "error" });
    } finally {
      setBusy(null);
    }
  };

  if (error && !app) {
    return <ErrorState title={error.message === "App not found." ? `No app named ${name}` : "Could not load the app"} description={error.message} onRetry={reload} />;
  }
  const [label, tone] = app ? headerBadge(app) : ["", "neutral" as Tone];

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-4">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="flex min-w-0 flex-col gap-2.5">
            <div className="flex flex-wrap items-center gap-3">
              <h1 className="text-22 font-semibold lg:text-28">{name}</h1>
              {app && <Badge tone={tone}>{label}</Badge>}
              {app && <Tag>{runtimeLabel(app.runtime)}</Tag>}
            </div>
            {app && (
              <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-13 text-ink-2">
                <span className="inline-flex items-center gap-1.5">
                  <BranchIcon size={14} className="text-ink-3" />
                  <span className="font-mono">
                    {shortRepo(app.repo)} · {app.branch}
                  </span>
                </span>
                <span className="font-mono">{app.path}</span>
                {app.port && <span className="font-mono">127.0.0.1:{app.port}</span>}
                {app.domain && (
                  <a href={`https://${app.domain}`} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 font-mono text-info">
                    {app.domain}
                    <ExternalIcon size={13} />
                  </a>
                )}
              </div>
            )}
          </div>
          {app && (
            <div className="flex flex-wrap gap-2">
              <Button variant="secondary" icon={<RestartIcon />} busy={busy === "restart"} disabled={app.release == null || deploying} onClick={() => run("restart")}>
                {busy === "restart" ? "Restarting…" : "Restart"}
              </Button>
              {deploying ? (
                <Button busy disabled>
                  Deploying…
                </Button>
              ) : (
                <Button icon={<DeployIcon />} busy={busy === "deploy"} onClick={() => run("deploy")}>
                  {lastFailed(app) ? "Retry deploy" : `Deploy ${app.branch}`}
                </Button>
              )}
            </div>
          )}
        </div>
        <nav aria-label="App sections" className="flex gap-1 overflow-x-auto border-b border-line">
          {TABS.map((t) => {
            const on = t.key === tab;
            return (
              <a
                key={t.key}
                href={`#/apps/${encodeURIComponent(name)}${t.suffix}`}
                aria-current={on ? "page" : undefined}
                className={`-mb-px inline-flex min-h-11 items-center gap-2 whitespace-nowrap border-b-2 px-3.5 text-sm no-underline ${
                  on ? "border-ink font-semibold text-ink" : "border-transparent font-medium text-ink-2 hover:text-ink"
                }`}
              >
                {t.label}
              </a>
            );
          })}
        </nav>
      </div>
      {tab === "env" ? (
        <AppEnvTab name={name} />
      ) : !app ? (
        <SkeletonBlock />
      ) : tab === "settings" ? (
        <AppSettingsTab app={app} reload={reload} />
      ) : (
        <AppOverviewTab app={app} reload={reload} />
      )}
    </div>
  );
}
