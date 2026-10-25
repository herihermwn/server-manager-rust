import { useEffect, useState } from "react";
import { api, ApiError } from "../../lib/api";
import { formatWib } from "../../lib/server";
import { Button } from "../../components/Button";
import { InlineAlert } from "../../components/Feedback";
import { Checkbox, Select, TextField, Textarea } from "../../components/Input";
import { LogView } from "../../components/LogView";
import { AlertCircleIcon, CheckIcon } from "../../components/icons";
import { CopyKeyButton, DeployKeyCard } from "../../components/DeployKeyCard";
import { DeployPanel, PipelineSteps, type StepItem } from "../../components/DeployPanel";
import type { DeployDetail } from "../../components/appsApi";

type Step = "repo" | "config" | "running" | "failed";

interface RepoCheck {
  branches: { name: string; commit: string }[];
  defaultBranch: string | null;
  headCommit: string | null;
}

interface Detect {
  runtime: string;
  detectedFrom: string;
  buildCmd: string;
  startCmd: string;
  commit: { sha: string; message: string; author: string; time: number } | null;
  envExample: number | null;
  suggestedPort: number;
  defaultRunAs: string;
}

const RUNTIMES = [
  { value: "rust", label: "Rust", hint: "cargo" },
  { value: "bun", label: "Bun", hint: "bun install" },
  { value: "node", label: "Node.js", hint: "npm ci" },
  { value: "static", label: "Static", hint: "files + small server" },
  { value: "other", label: "Custom", hint: "your commands" },
];

const nameFromRepo = (r: string) =>
  (r.split(/[/:]/).pop() ?? "")
    .replace(/\.git$/, "")
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);

function Ok({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-1.5 text-13 text-success">
      <CheckIcon size={15} strokeWidth={2.5} />
      <span>{children}</span>
    </div>
  );
}

function Bad({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-1.5 text-13 text-danger">
      <AlertCircleIcon size={15} strokeWidth={2.25} />
      <span>{children}</span>
    </div>
  );
}

/** AppNew.dc.html: repository → folder & commands → clone & first deploy (or clone refused). */
export function AppNew() {
  const [step, setStep] = useState<Step>("repo");
  const [repo, setRepo] = useState("");
  const [check, setCheck] = useState<RepoCheck | null>(null);
  const [checkErr, setCheckErr] = useState("");
  const [checking, setChecking] = useState(false);
  const [branch, setBranch] = useState("");
  const [det, setDet] = useState<Detect | null>(null);
  const [detErr, setDetErr] = useState("");
  const [detecting, setDetecting] = useState(false);

  const [name, setName] = useState("");
  const [folder, setFolder] = useState<{ path: string; free: boolean; message: string } | null>(null);
  const [runtime, setRuntime] = useState("other");
  const [build, setBuild] = useState("");
  const [start, setStart] = useState("");
  const [port, setPort] = useState("");
  const [portMsg, setPortMsg] = useState<{ free: boolean; message: string } | null>(null);
  const [health, setHealth] = useState("/");
  const [runAs, setRunAs] = useState("");
  const [envExample, setEnvExample] = useState(true);
  const [formErr, setFormErr] = useState("");

  const [deployId, setDeployId] = useState<number | null>(null);
  const [cloneErr, setCloneErr] = useState<{ error: string; log: string[] } | null>(null);
  const [cloneStart, setCloneStart] = useState(0);
  const [finished, setFinished] = useState<DeployDetail | null>(null);

  const testAccess = async () => {
    setChecking(true);
    setCheckErr("");
    setCheck(null);
    setDet(null);
    try {
      const r = await api<{ data: RepoCheck }>("/api/apps/check-repo", { body: { repo: repo.trim() } });
      setCheck(r.data);
      setBranch(r.data.defaultBranch ?? r.data.branches[0]?.name ?? "");
      if (!name) setName(nameFromRepo(repo.trim()));
    } catch (e) {
      setCheckErr((e as Error).message);
    } finally {
      setChecking(false);
    }
  };

  // Branch picked → shallow look at the files for runtime + commands + latest commit.
  useEffect(() => {
    if (!check || !branch) return;
    let live = true;
    setDetecting(true);
    setDetErr("");
    api<{ data: Detect }>("/api/apps/detect", { body: { repo: repo.trim(), branch } })
      .then((r) => {
        if (!live) return;
        setDet(r.data);
        setRuntime(r.data.runtime);
        setBuild(r.data.buildCmd);
        setStart(r.data.startCmd);
        setPort((p) => p || String(r.data.suggestedPort));
        setRunAs((u) => u || r.data.defaultRunAs);
      })
      .catch((e) => live && setDetErr((e as Error).message))
      .finally(() => live && setDetecting(false));
    return () => {
      live = false;
    };
  }, [check, branch]);

  // Folder = <apps>/<name>; check it as the name changes.
  useEffect(() => {
    if (step !== "config" || !name) return setFolder(null);
    const t = setTimeout(() => {
      api<{ data: { path: string; free: boolean; message: string } }>("/api/apps/check-folder", { body: { name } })
        .then((r) => setFolder(r.data))
        .catch((e) => setFolder({ path: "", free: false, message: (e as Error).message }));
    }, 300);
    return () => clearTimeout(t);
  }, [name, step]);

  useEffect(() => {
    const p = Number(port);
    if (step !== "config" || !p) return setPortMsg(null);
    const t = setTimeout(() => {
      api<{ data: { free: boolean; message: string } }>("/api/apps/check-port", { body: { port: p } })
        .then((r) => setPortMsg(r.data))
        .catch((e) => setPortMsg({ free: false, message: (e as Error).message }));
    }, 300);
    return () => clearTimeout(t);
  }, [port, step]);

  const create = async () => {
    setFormErr("");
    setCloneErr(null);
    setStep("running");
    setCloneStart(Date.now());
    try {
      const r = await api<{ data: { name: string; deployId: number } }>("/api/apps", {
        body: {
          name,
          repo: repo.trim(),
          branch,
          runtime,
          buildCmd: build,
          startCmd: start,
          port: Number(port),
          healthPath: health,
          envFromExample: envExample && !!det?.envExample,
          runAs: runAs.trim(),
        },
      });
      setDeployId(r.data.deployId);
    } catch (e) {
      const body = e instanceof ApiError ? (e.body as { log?: string[] }) : {};
      if (body.log) {
        setCloneErr({ error: (e as Error).message, log: body.log });
        setStep("failed");
      } else {
        setFormErr((e as Error).message);
        setStep("config");
      }
    }
  };

  const idx = { repo: 0, config: 1, running: 2, failed: 2 }[step];
  const wizardSteps = [
    { label: "Repository", hint: "Git link and branch" },
    { label: "Folder & commands", hint: "Where and how it runs" },
    { label: "Clone & deploy", hint: step === "failed" ? "Clone failed" : "First release" },
  ];
  const canNext = !!check && !!branch && !detecting;
  const canCreate = !!name && folder?.free && start.trim() && portMsg?.free && runAs.trim();
  const shortRepo = repo.trim().replace(/^git@[^:]+:/, "").replace(/\.git$/, "");

  const clonePipeline = (failed: boolean): StepItem[] => [
    { label: "Check folder", cmd: folder?.path, time: "0s", state: "done" },
    { label: "Clone", cmd: `git clone --bare -b ${branch}`, time: failed ? `${Math.round((Date.now() - cloneStart) / 1000)}s` : "", state: failed ? "fail" : "run" },
    { label: "Create .env", cmd: envExample && det?.envExample ? `${det.envExample} keys from .env.example` : "PORT only", state: "wait" },
    { label: "Build", cmd: build.split("\n")[0] || "no build command", state: "wait" },
    { label: "Start", cmd: `systemctl start app-${name}`, state: "wait" },
    { label: "Health check", cmd: health ? `GET :${port}${health}` : "process stays up", state: "wait" },
  ];

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-1">
        <h1 className="text-22 font-semibold lg:text-28">New app</h1>
        <p className="text-sm text-ink-2">Clone a repository onto this server, build it and keep it running.</p>
      </div>
      <ol aria-label="Steps" className="m-0 grid list-none grid-cols-1 gap-3 p-0 sm:grid-cols-3">
        {wizardSteps.map((s, i) => {
          const done = i < idx;
          const cur = i === idx;
          return (
            <li
              key={s.label}
              aria-current={cur ? "step" : undefined}
              className={`flex items-center gap-3 rounded-xl px-4 py-3 ${cur ? "border border-ink bg-surface" : "border border-line"}`}
            >
              <span
                className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full font-mono text-13 font-semibold ${
                  done ? "bg-success text-ink-on" : cur ? (step === "failed" ? "bg-danger text-ink-on" : "bg-ink text-ink-on") : "bg-muted text-ink-2"
                }`}
              >
                {done ? "✓" : i + 1}
              </span>
              <span className="flex flex-col">
                <span className="text-sm font-semibold">{s.label}</span>
                <span className="text-xs text-ink-3">{s.hint}</span>
              </span>
            </li>
          );
        })}
      </ol>

      <div className="flex flex-wrap items-start gap-6">
        <div className="flex min-w-0 flex-[999_1_560px] flex-col gap-6">
          {step === "repo" && (
            <section className="flex flex-col gap-5 rounded-xl border border-line bg-surface p-6">
              <h2 className="text-17 font-semibold">Repository</h2>
              <div className="flex flex-col gap-1.5">
                <label htmlFor="repo" className="text-13 font-semibold">
                  Git link
                </label>
                <form
                  className="flex flex-wrap gap-2"
                  onSubmit={(e) => {
                    e.preventDefault();
                    if (repo.trim()) testAccess();
                  }}
                >
                  <input
                    id="repo"
                    value={repo}
                    onChange={(e) => {
                      setRepo(e.target.value);
                      setCheck(null);
                      setCheckErr("");
                    }}
                    placeholder="git@github.com:you/app.git"
                    spellCheck={false}
                    autoComplete="off"
                    className={`h-11 min-w-0 flex-[1_1_320px] rounded-lg bg-surface px-3 font-mono text-sm text-ink placeholder:text-ink-3 ${checkErr ? "border-[1.5px] border-danger" : "border border-line-strong"}`}
                  />
                  <Button type="submit" variant="subtle" busy={checking} disabled={!repo.trim()}>
                    {checking ? "Testing…" : "Test access"}
                  </Button>
                </form>
                {check && (
                  <Ok>
                    Reachable with the server deploy key. {check.branches.length} {check.branches.length === 1 ? "branch" : "branches"}, default{" "}
                    <span className="font-mono">{check.defaultBranch ?? "—"}</span> at <span className="font-mono">{check.headCommit?.slice(0, 7) ?? "—"}</span>.
                  </Ok>
                )}
                {checkErr && <Bad>{checkErr}</Bad>}
                <div className="text-xs text-ink-3">SSH links use the deploy key. HTTPS links work for public repositories.</div>
              </div>
              {check && (
                <div className="grid gap-4 sm:grid-cols-[minmax(0,240px)_1fr]">
                  <Select label="Branch to deploy" value={branch} onChange={(e) => setBranch(e.target.value)} options={check.branches.map((b) => ({ value: b.name, label: b.name }))} />
                  <div className="flex min-w-0 flex-col gap-1.5">
                    <span className="text-13 font-semibold">Latest commit</span>
                    <div className="flex min-h-11 flex-col justify-center rounded-lg border border-line bg-sunken px-3 py-2 text-13">
                      {detecting ? (
                        <span className="text-ink-3">Reading {branch}…</span>
                      ) : det?.commit ? (
                        <>
                          <span className="truncate">
                            <span className="font-mono">{det.commit.sha}</span> {det.commit.message}
                          </span>
                          <span className="text-xs text-ink-3">
                            {det.commit.author} · {formatWib(det.commit.time)}
                          </span>
                        </>
                      ) : (
                        <span className="text-danger">{detErr || "—"}</span>
                      )}
                    </div>
                  </div>
                </div>
              )}
              <div className="flex flex-wrap justify-end gap-2 border-t border-line-soft pt-4">
                <Button variant="secondary" href="#/apps">
                  Cancel
                </Button>
                <Button disabled={!canNext} onClick={() => setStep("config")}>
                  Next: folder &amp; commands
                </Button>
              </div>
            </section>
          )}

          {step === "config" && (
            <section className="flex flex-col gap-5 rounded-xl border border-line bg-surface p-6">
              {formErr && <InlineAlert title="The app was not created">{formErr}</InlineAlert>}
              <h2 className="text-17 font-semibold">Folder</h2>
              <div className="grid gap-4 sm:grid-cols-2">
                <TextField
                  label="App name"
                  mono
                  value={name}
                  onChange={(e) => setName(e.target.value.toLowerCase())}
                  maxLength={40}
                  helper="Lowercase and dashes. Also the systemd unit app-<name>.service."
                  error={name && !/^[a-z0-9][a-z0-9-]*$/.test(name) ? "Use a-z, 0-9 and -, not starting with -." : undefined}
                />
                <div className="flex min-w-0 flex-col gap-1.5">
                  <span className="text-13 font-semibold">Clone into</span>
                  <div className="flex h-11 items-center truncate rounded-lg border border-line bg-paper px-3 font-mono text-sm text-ink-2">{folder?.path || "—"}</div>
                  {folder && (folder.free ? <Ok>{folder.message}{runAs ? `, owned by ${runAs}` : ""}</Ok> : <Bad>{folder.message}</Bad>)}
                </div>
              </div>
              <div className="h-px bg-line-soft" />
              <h2 className="text-17 font-semibold">Build &amp; run</h2>
              <fieldset className="m-0 flex flex-col gap-2 border-0 p-0">
                <legend className="mb-2 text-13 font-semibold">
                  Runtime {det?.detectedFrom && <span className="font-normal text-ink-3">· detected from {det.detectedFrom}</span>}
                </legend>
                <div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
                  {RUNTIMES.map((r) => {
                    const on = runtime === r.value;
                    return (
                      <label
                        key={r.value}
                        className={`relative flex min-h-16 cursor-pointer flex-col gap-0.5 rounded-[10px] ${on ? "border-2 border-ink bg-sunken p-[11px]" : "border border-line-strong bg-surface p-3"}`}
                      >
                        <input type="radio" name="rt" checked={on} onChange={() => setRuntime(r.value)} className="sr-only" />
                        <span className="text-sm font-semibold">{r.label}</span>
                        <span className="font-mono text-xs text-ink-3">{r.hint}</span>
                      </label>
                    );
                  })}
                </div>
              </fieldset>
              <Textarea
                label="Build command"
                rows={3}
                value={build}
                onChange={(e) => setBuild(e.target.value)}
                className="font-mono"
                optional
                helper="Runs in the release folder on every deploy. One command per line, stops at the first failure."
              />
              <div className="grid gap-4 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1fr)]">
                <TextField label="Start command" mono value={start} onChange={(e) => setStart(e.target.value)} helper="Runs under systemd with .env loaded." />
                <TextField
                  label="Port"
                  mono
                  inputMode="numeric"
                  value={port}
                  onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
                  error={portMsg && !portMsg.free ? portMsg.message : undefined}
                  helper={portMsg?.free ? portMsg.message : undefined}
                />
                <TextField label="Health check" optional mono value={health} onChange={(e) => setHealth(e.target.value)} placeholder="/health" helper="2xx within 30 s" />
              </div>
              <TextField
                label="Run as user"
                mono
                value={runAs}
                onChange={(e) => setRunAs(e.target.value.trim())}
                wrapperClassName="sm:max-w-xs"
                helper={runAs === "root" ? "Runs with full access to the server. Prefer a normal user." : "Build, start and the files belong to this system user."}
              />
              <div className="h-px bg-line-soft" />
              <h2 className="text-17 font-semibold">Also set up</h2>
              <Checkbox
                checked={envExample && !!det?.envExample}
                disabled={!det?.envExample}
                onChange={(e) => setEnvExample(e.target.checked)}
                label={
                  <span className="flex flex-col">
                    <span>Create .env from .env.example</span>
                    <span className="text-13 font-normal text-ink-2">
                      {det?.envExample
                        ? `${det.envExample} keys found. Fill the empty ones on the Environment tab.`
                        : "No .env.example in this branch. An .env with PORT is created."}
                    </span>
                  </span>
                }
              />
              <p className="text-13 text-ink-3">Databases and nginx sites are added from their own pages once the app exists.</p>
              <div className="flex flex-wrap justify-end gap-2 border-t border-line-soft pt-4">
                <Button variant="secondary" onClick={() => setStep("repo")}>
                  Back
                </Button>
                <Button disabled={!canCreate} onClick={create}>
                  Clone &amp; deploy
                </Button>
              </div>
            </section>
          )}

          {step === "failed" && cloneErr && (
            <div role="alert" className="flex gap-3 rounded-xl bg-danger-bg p-4 text-danger-ink">
              <AlertCircleIcon size={20} strokeWidth={2.25} className="mt-0.5 shrink-0 text-danger" />
              <div className="flex flex-col gap-2 text-sm">
                <strong className="font-semibold">{cloneErr.error.startsWith("Permission denied") ? "GitHub refused the deploy key." : "Clone failed."}</strong>
                <span>
                  {cloneErr.error} Nothing was created in <span className="font-mono">{folder?.path}</span>.
                </span>
                <div className="flex flex-wrap gap-2 pt-1">
                  <Button onClick={create}>Retry clone</Button>
                  <CopyKeyButton />
                  <Button variant="secondary" onClick={() => setStep("repo")}>
                    Edit repository
                  </Button>
                </div>
              </div>
            </div>
          )}

          {(step === "running" || step === "failed") && !deployId && (
            <>
              <section className="flex flex-col gap-3 rounded-xl border border-line bg-surface p-5">
                <div className="flex items-baseline justify-between">
                  <h2 className="text-17 font-semibold">{step === "failed" ? "Clone failed" : `Cloning ${shortRepo}`}</h2>
                </div>
                <PipelineSteps steps={clonePipeline(step === "failed")} />
              </section>
              {cloneErr && <LogView title="Log · clone" lines={cloneErr.log.map((text) => ({ text }))} />}
            </>
          )}

          {deployId && (
            <>
              {finished?.status === "success" && (
                <div role="status" className="flex flex-wrap items-center gap-3 rounded-xl bg-success-bg px-4 py-3 text-sm text-success">
                  <CheckIcon size={20} strokeWidth={2.25} />
                  <span className="flex-1">
                    <strong>{name} is live.</strong> Release #{finished.number} passed the health check.
                  </span>
                  <Button href={`#/apps/${name}`}>Open app</Button>
                </div>
              )}
              {finished && finished.status !== "success" && (
                <InlineAlert title={`First deploy ${finished.status.replace("_", " ")}`}>
                  {finished.error} The app was created; fix it on the app page and deploy again. <a href={`#/apps/${name}`}>Open {name}</a>
                </InlineAlert>
              )}
              <DeployPanel app={name} deployId={deployId} onFinish={setFinished} />
              {!finished && (
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <span className="text-13 text-ink-2">You can leave this page. The deploy keeps running and shows in the topbar.</span>
                  <Button variant="subtle" href={`#/apps/${name}`}>
                    Open app page
                  </Button>
                </div>
              )}
            </>
          )}
        </div>

        <aside className="flex min-w-0 flex-[1_1_300px] flex-col gap-6">
          <DeployKeyCard />
          <section className="flex flex-col gap-3 rounded-xl border border-line bg-surface p-5">
            <h2 className="text-15 font-semibold">What will run</h2>
            <div className="flex flex-col gap-1 break-all rounded-lg bg-console px-3 py-3 font-mono text-xs leading-5 text-console-text">
              <span>
                <span className="text-console-muted">$</span> git clone --bare -b {branch || "main"} {shortRepo || "…"} ~/apps/{name || "<name>"}/repo
              </span>
              {envExample && det?.envExample ? (
                <span>
                  <span className="text-console-muted">$</span> cp .env.example shared/.env
                </span>
              ) : null}
              {build
                .split("\n")
                .filter(Boolean)
                .map((l, i) => (
                  <span key={i}>
                    <span className="text-console-muted">$</span> {l}
                  </span>
                ))}
              <span>
                <span className="text-console-muted">$</span> systemctl start app-{name || "<name>"}
              </span>
              {health && (
                <span>
                  <span className="text-console-muted">$</span> curl -f 127.0.0.1:{port || "<port>"}
                  {health}
                </span>
              )}
            </div>
            <p className="text-13 text-ink-2">
              Build and start run as <span className="font-mono">{runAs || "the app user"}</span>; git fetch and the systemd unit are handled by the manager.
            </p>
          </section>
        </aside>
      </div>
    </div>
  );
}
