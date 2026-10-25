import { useMemo, useState } from "react";
import { api } from "../../lib/api";
import { navigate } from "../../lib/router";
import { formatWib } from "../../lib/server";
import { StatusBadge } from "../../components/Badge";
import { Button } from "../../components/Button";
import { DataTable, type Column } from "../../components/DataTable";
import { ErrorState } from "../../components/Feedback";
import { FilterTabs } from "../../components/FilterTabs";
import { PageHeader } from "../../components/PageHeader";
import { useToast } from "../../components/Toast";
import { AppsIcon, PlusIcon, SearchIcon } from "../../components/icons";
import { CopyKeyButton } from "../../components/DeployKeyCard";
import { appUrl, fmtBytes, runtimeLabel, useApi, type AppSummary } from "../../components/appsApi";

type Filter = "all" | "running" | "stopped" | "crashed";

const STEPS = [
  { title: "Add the deploy key", body: "Paste the server public key into GitHub › Settings › Deploy keys. Read-only is enough." },
  { title: "Paste the repository", body: "SSH link like git@github.com:you/app.git, and the branch to deploy." },
  { title: "Pick folder and commands", body: "How to build and start, which port. Then clone and deploy." },
];

function EmptyApps() {
  return (
    <section className="flex flex-col items-center gap-6 rounded-xl border border-line bg-surface px-8 py-12 text-center">
      <div className="flex flex-col items-center gap-2">
        <div className="flex h-[52px] w-[52px] items-center justify-center rounded-[14px] bg-muted text-ink-2">
          <AppsIcon size={24} />
        </div>
        <h2 className="mt-1 text-xl font-semibold">No apps on this server yet</h2>
        <p className="max-w-[520px] text-sm leading-[21px] text-ink-2">
          Add the server deploy key to your git host once, then paste a repository link. The server clones it and runs the first deploy.
        </p>
      </div>
      <ol className="m-0 grid w-full max-w-[860px] list-none grid-cols-[repeat(auto-fit,minmax(220px,1fr))] gap-3 p-0 text-left">
        {STEPS.map((s, i) => (
          <li key={s.title} className="flex flex-col gap-1.5 rounded-xl border border-line bg-sunken p-4">
            <span className="flex h-[26px] w-[26px] items-center justify-center rounded-full bg-ink font-mono text-13 font-semibold text-ink-on">{i + 1}</span>
            <span className="text-sm font-semibold">{s.title}</span>
            <span className="text-13 leading-[19px] text-ink-2">{s.body}</span>
          </li>
        ))}
      </ol>
      <div className="flex flex-wrap justify-center gap-2">
        <CopyKeyButton />
        <Button href="#/apps/new">New app</Button>
      </div>
    </section>
  );
}

const shortRepo = (r: string) => r.replace(/^(git@[^:]+:|https?:\/\/[^/]+\/|ssh:\/\/[^/]+\/)/, "").replace(/\.git$/, "");

/** Apps.dc.html: filter tabs + search, table with status, release, domain, CPU · mem, last deploy. */
export function Apps() {
  const { data, error, loading, reload } = useApi<{ data: AppSummary[] }>("/api/apps", 5000);
  const apps = data?.data ?? [];
  const [filter, setFilter] = useState<Filter>("all");
  const [q, setQ] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const toast = useToast();

  const count = (f: Filter) => apps.filter((a) => f === "all" || a.status === f || (f === "running" && a.status === "deploying")).length;
  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return apps.filter(
      (a) =>
        (filter === "all" || a.status === filter || (filter === "running" && a.status === "deploying")) &&
        (!needle || [a.name, a.repo, a.domain ?? ""].some((s) => s.toLowerCase().includes(needle))),
    );
  }, [apps, filter, q]);

  const act = async (a: AppSummary, what: "deploy" | "start") => {
    setBusy(a.name);
    try {
      if (what === "deploy") {
        await api(`${appUrl(a.name)}/deploys`, { method: "POST" });
        navigate(`/apps/${a.name}`);
      } else {
        await api(`${appUrl(a.name)}/start`, { method: "POST" });
        toast({ title: `${a.name} started`, tone: "success" });
        reload();
      }
    } catch (e) {
      toast({ title: what === "deploy" ? "Deploy not started" : "Could not start", body: (e as Error).message, tone: "error" });
    } finally {
      setBusy(null);
    }
  };

  const columns: Column<AppSummary>[] = [
    {
      key: "name",
      header: "App",
      render: (a) => (
        <span className="flex flex-col">
          <span className="font-semibold text-ink">{a.name}</span>
          <span className="flex gap-2 text-xs font-normal text-ink-3">
            <span>{runtimeLabel(a.runtime)}</span>
            <span className="font-mono">{shortRepo(a.repo)}</span>
          </span>
        </span>
      ),
    },
    { key: "status", header: "Status", render: (a) => <StatusBadge status={a.status} /> },
    { key: "release", header: "Release", mono: true, render: (a) => <span className="whitespace-nowrap text-ink-2">{a.commit ? `${a.branch} · ${a.commit}` : "—"}</span> },
    { key: "domain", header: "Domain", mono: true, render: (a) => <span className="text-ink-2">{a.domain ?? "—"}</span> },
    {
      key: "res",
      header: "CPU · Mem",
      align: "right",
      mono: true,
      render: (a) => (a.status === "running" || a.status === "deploying" ? `${a.cpuPct == null ? "—" : `${a.cpuPct}%`} · ${fmtBytes(a.memBytes)}` : "—"),
    },
    {
      key: "last",
      header: "Last deploy",
      render: (a) =>
        a.lastDeploy ? (
          <span className={`whitespace-nowrap text-13 ${a.lastDeploy.status === "failed" || a.lastDeploy.status === "rolled_back" ? "text-danger" : "text-ink-2"}`}>
            #{a.lastDeploy.number} · {a.lastDeploy.finishedAt ? formatWib(a.lastDeploy.finishedAt) : a.lastDeploy.status}
            {a.lastDeploy.status === "failed" ? " · failed" : a.lastDeploy.status === "rolled_back" ? " · rolled back" : ""}
          </span>
        ) : (
          <span className="text-13 text-ink-3">Never</span>
        ),
    },
    {
      key: "act",
      header: <span className="sr-only">Actions</span>,
      align: "right",
      render: (a) =>
        a.status === "stopped" && a.release != null ? (
          <Button size="sm" variant="secondary" busy={busy === a.name} onClick={() => act(a, "start")}>
            Start
          </Button>
        ) : (
          <Button size="sm" busy={busy === a.name} disabled={a.status === "deploying"} onClick={() => act(a, "deploy")}>
            {a.status === "deploying" ? "Deploying…" : "Deploy"}
          </Button>
        ),
    },
  ];

  const running = apps.filter((a) => a.status === "running").length;
  const ports = apps.map((a) => a.port).filter((p): p is number => p != null).sort((a, b) => a - b);

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Apps"
        subtitle="Each app is a git checkout in its own folder with a build command, a systemd process and an optional nginx site."
        actions={
          <Button href="#/apps/new" icon={<PlusIcon />}>
            New app
          </Button>
        }
      />
      {error && !data ? (
        <ErrorState title="Could not load apps" description={error.message} onRetry={reload} />
      ) : !loading && apps.length === 0 ? (
        <EmptyApps />
      ) : (
        <>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <FilterTabs<Filter>
              label="App status"
              value={filter}
              onChange={setFilter}
              options={[
                { value: "all", label: "All", count: count("all") },
                { value: "running", label: "Running", count: count("running") },
                { value: "stopped", label: "Stopped", count: count("stopped") },
                { value: "crashed", label: "Crashed", count: count("crashed") },
              ]}
            />
            <form role="search" onSubmit={(e) => e.preventDefault()} className="flex h-11 flex-[0_1_320px] items-center gap-2 rounded-lg border border-line-strong bg-surface px-3">
              <SearchIcon className="text-ink-3" />
              <label htmlFor="apps-q" className="sr-only">
                Search apps
              </label>
              <input
                id="apps-q"
                value={q}
                onChange={(e) => setQ(e.target.value)}
                placeholder="Search name, repo or domain"
                className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-ink-3"
              />
            </form>
          </div>
          <section className="overflow-hidden rounded-xl border border-line bg-surface">
            <DataTable
              columns={columns}
              data={shown}
              loading={loading}
              rowKey={(a) => a.name}
              rowHref={(a) => `#/apps/${a.name}`}
              framed={false}
              minWidth={1000}
              label="Apps"
              emptyMessage={q ? `No app matches "${q.trim()}"` : "No apps with this status"}
            />
            {!loading && (
              <div className="border-t border-line px-5 py-3 text-13 text-ink-2">
                {apps.length} {apps.length === 1 ? "app" : "apps"} · {running} running
                {ports.length > 0 && (
                  <>
                    {" "}
                    · ports in use <span className="font-mono text-ink">{ports.join(", ")}</span>
                  </>
                )}
              </div>
            )}
          </section>
        </>
      )}
    </div>
  );
}
