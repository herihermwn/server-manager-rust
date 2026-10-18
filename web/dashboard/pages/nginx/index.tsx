import { useCallback, useEffect, useState } from "react";
import { api, ApiError } from "../../lib/api";
import { formatWib } from "../../lib/server";
import { Badge } from "../../components/Badge";
import { Button } from "../../components/Button";
import { KpiStrip } from "../../components/Card";
import { DataTable, type Column } from "../../components/DataTable";
import { Banner, EmptyState, ErrorState, InlineAlert } from "../../components/Feedback";
import { PlusIcon } from "../../components/icons";
import { KIND_LABEL, sslLabel, type NginxSite, type NginxStatus, type NginxTest } from "../../components/NginxApi";
import { PageHeader } from "../../components/PageHeader";
import { useToast } from "../../components/Toast";

/** Web server: nginx status, certificate expiry, all sites. Design: Nginx */
export function Nginx() {
  const toast = useToast();
  const [sites, setSites] = useState<NginxSite[] | null>(null);
  const [status, setStatus] = useState<NginxStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"test" | "reload" | null>(null);
  const [failed, setFailed] = useState<{ title: string; output: string } | null>(null);

  const load = useCallback(() => {
    setError(null);
    Promise.all([api<NginxSite[]>("/api/sites"), api<NginxStatus>("/api/nginx/status")])
      .then(([s, st]) => {
        setSites(s);
        setStatus(st);
      })
      .catch((e: ApiError) => setError(e.message));
  }, []);
  useEffect(load, [load]);

  async function run(kind: "test" | "reload") {
    setBusy(kind);
    setFailed(null);
    try {
      if (kind === "test") {
        const t = await api<NginxTest>("/api/nginx/test", { method: "POST" });
        if (t.ok) toast({ title: "Config test passed", body: "nginx -t is OK.", tone: "success" });
        else setFailed({ title: "nginx -t failed. Nothing was reloaded.", output: t.output });
      } else {
        await api("/api/nginx/reload", { method: "POST" });
        toast({ title: "nginx reloaded", body: "nginx -t passed first.", tone: "success" });
      }
    } catch (e) {
      const err = e as ApiError;
      const out = (err.body?.nginx as { output?: string } | undefined)?.output ?? "";
      setFailed({ title: err.message, output: out });
    } finally {
      setBusy(null);
      api<NginxStatus>("/api/nginx/status").then(setStatus).catch(() => {});
    }
  }

  const columns: Column<NginxSite>[] = [
    {
      key: "domain",
      header: "Domain",
      render: (s) => (
        <div className="py-2.5">
          <a href={`#/nginx/${s.id}`} className="font-mono text-sm font-semibold text-ink no-underline hover:underline">
            {s.domain}
          </a>
          {s.aliases.length > 0 && <div className="font-mono text-xs text-ink-3">{s.aliases.join(", ")}</div>}
        </div>
      ),
    },
    {
      key: "serves",
      header: "Serves",
      render: (s) =>
        s.kind === "proxy" ? (
          <span className="text-13">
            <span className="font-medium">{s.appName ?? "No app"}</span> <span className="font-mono text-ink-2">→ {s.upstream ?? "no port"}</span>
          </span>
        ) : s.kind === "static" ? (
          <span className="text-13">
            <span className="font-medium">{KIND_LABEL.static}</span> <span className="font-mono text-ink-2">{s.rootPath}</span>
          </span>
        ) : (
          <span className="text-13">
            <span className="font-medium">{KIND_LABEL.redirect}</span> <span className="font-mono text-ink-2">→ {s.redirectUrl}</span>
          </span>
        ),
    },
    {
      key: "ssl",
      header: "SSL",
      render: (s) => {
        const l = sslLabel(s);
        return <Badge tone={l.tone}>{l.text}</Badge>;
      },
    },
    {
      key: "enabled",
      header: "Site",
      render: (s) => <span className={`text-13 font-medium ${s.enabled ? "text-success" : "text-ink-3"}`}>{s.enabled ? "Enabled" : "Disabled"}</span>,
    },
    {
      key: "actions",
      header: <span className="sr-only">Actions</span>,
      align: "right",
      render: (s) => (
        <span className="inline-flex gap-1">
          <Button variant="subtle" size="sm" href={`${s.certReady && s.ssl ? "https" : "http"}://${s.domain}`} target="_blank" rel="noreferrer">
            Open
          </Button>
          <Button variant="secondary" size="sm" href={`#/nginx/${s.id}`}>
            Edit
          </Button>
        </span>
      ),
    },
  ];

  const last = status?.lastTest;
  const expiring = status?.certs.expiring ?? [];

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Web server"
        subtitle={
          <>
            nginx sites in <span className="font-mono">{status?.dir ?? "/etc/nginx/sites-manager"}</span>. Every save runs <span className="font-mono">nginx -t</span> before
            reload, so a broken config never goes live.
          </>
        }
        actions={
          <>
            <Button variant="secondary" busy={busy === "test"} disabled={busy !== null} onClick={() => run("test")}>
              {busy === "test" ? "Testing…" : "Test config"}
            </Button>
            <Button variant="secondary" busy={busy === "reload"} disabled={busy !== null} onClick={() => run("reload")}>
              {busy === "reload" ? "Reloading…" : "Reload nginx"}
            </Button>
            <Button href="#/nginx/new" icon={<PlusIcon size={18} />}>
              Add site
            </Button>
          </>
        }
      />

      {error && !sites ? (
        <ErrorState title="Could not load the web server" description={`${error} Nothing was changed.`} onRetry={load} />
      ) : (
        <>
          <KpiStrip
            loading={!status}
            items={[
              {
                label: "nginx",
                value: (
                  <span className="inline-flex items-center gap-2">
                    <span className={`h-2 w-2 rounded-full ${status?.active ? "bg-success" : "bg-danger"}`} aria-hidden="true" />
                    {status?.active ? "Active" : "Not running"}
                  </span>
                ),
                meta: status?.version ? `v${status.version}` : "Version unknown",
              },
              {
                label: "Config test",
                value: last ? (last.ok ? "OK" : "Failed") : "Not run",
                meta: last ? `nginx -t · ${formatWib(last.at, { timeStyle: "short" })}` : "Runs on every save",
              },
              { label: "Sites", value: `${status?.sites.enabled ?? 0} of ${status?.sites.total ?? 0}`, meta: "enabled" },
              {
                label: "Certificates",
                value: status?.certs.total ?? 0,
                meta: expiring.length ? `${expiring.length} expire${expiring.length === 1 ? "s" : ""} within 30 days` : "Let’s Encrypt",
              },
            ]}
          />

          {failed && (
            <InlineAlert title={failed.title}>
              {failed.output && <pre className="mt-2 whitespace-pre-wrap rounded-lg bg-surface p-3 font-mono text-[12.5px] leading-5">{failed.output}</pre>}
            </InlineAlert>
          )}

          {expiring.slice(0, 3).map((c) => (
            <Banner
              key={c.id}
              tone="warning"
              title={`SSL for ${c.domain} ${c.daysLeft < 0 ? "has expired" : c.daysLeft === 0 ? "expires today" : `expires in ${c.daysLeft} ${c.daysLeft === 1 ? "day" : "days"}`}.`}
              action={{ label: "Renew certificate", href: `#/nginx/${c.id}` }}
            >
              certbot renews on its own every night. If it did not, check that the A record points to <span className="font-mono">{status?.publicIp}</span>.
            </Banner>
          ))}

          <DataTable
            label="Sites"
            columns={columns}
            data={sites ?? []}
            loading={!sites}
            rowKey={(s) => s.id}
            minWidth={880}
            empty={
              <EmptyState
                title="No sites yet"
                description="Add a domain, pick the app it should serve, and the manager writes the nginx config, tests it and reloads."
                action={
                  <Button href="#/nginx/new" icon={<PlusIcon size={18} />}>
                    Add site
                  </Button>
                }
              />
            }
          />
        </>
      )}
    </div>
  );
}
