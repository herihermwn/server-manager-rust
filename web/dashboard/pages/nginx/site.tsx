import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, ApiError } from "../../lib/api";
import { navigate, parseRoute, useHashRoute } from "../../lib/router";
import { formatWib } from "../../lib/server";
import { Badge } from "../../components/Badge";
import { Button } from "../../components/Button";
import { EmptyState, ErrorState, InlineAlert, SkeletonBlock } from "../../components/Feedback";
import { Checkbox, Select, TextField, Textarea, Toggle } from "../../components/Input";
import { usePageCrumb } from "../../components/Layout";
import { ConfirmDialog } from "../../components/Modal";
import {
  emergMessage,
  emptyForm,
  formFromSite,
  KIND_LABEL,
  splitAliases,
  sslLabel,
  type NginxAppOption,
  type NginxDns,
  type NginxForm,
  type NginxSite,
} from "../../components/NginxApi";
import { NginxConfigPreview } from "../../components/NginxConfigPreview";
import { NginxSslProgress } from "../../components/NginxSslProgress";
import { PageHeader } from "../../components/PageHeader";
import { useToast } from "../../components/Toast";
import { CheckIcon, AlertTriangleIcon } from "../../components/icons";

interface Failure {
  message: string;
  output: string;
  line: number | null;
}

const KINDS: { value: NginxForm["kind"]; label: string; hint: string }[] = [
  { value: "proxy", label: "Proxy to app", hint: "nginx → local port" },
  { value: "static", label: "Static folder", hint: "serve files from a folder" },
  { value: "redirect", label: "Redirect", hint: "301 to another URL" },
];

/** Site editor. `id` is the site id, or "new". Design: NginxSite, NginxTestFailed, NginxSsl */
export function NginxSite({ id }: { id: string }) {
  const isNew = id === "new";
  const toast = useToast();
  const route = useHashRoute();
  const [site, setSite] = useState<NginxSite | null>(null);
  const [form, setForm] = useState<NginxForm>(emptyForm);
  const [aliasText, setAliasText] = useState("");
  const [apps, setApps] = useState<NginxAppOption[]>([]);
  const [loadError, setLoadError] = useState<{ message: string; status: number } | null>(null);
  const [loading, setLoading] = useState(!isNew);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [dns, setDns] = useState<NginxDns | null>(null);
  const [dnsBusy, setDnsBusy] = useState(false);
  const [preview, setPreview] = useState<{ config: string | null; error: string | null; sslActive: boolean }>({ config: null, error: null, sslActive: false });
  const [sslRun, setSslRun] = useState(parseRoute(route).query.get("ssl") === "1");
  const [toggling, setToggling] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteCert, setDeleteCert] = useState(false);
  usePageCrumb(isNew ? "New site" : site?.domain);

  const apply = useCallback((s: NginxSite) => {
    setSite(s);
    setForm(formFromSite(s));
    setAliasText(s.aliases.join(", "));
  }, []);

  const load = useCallback(() => {
    setLoadError(null);
    api<NginxAppOption[]>("/api/sites/apps").then(setApps).catch(() => {});
    if (isNew) return;
    setLoading(true);
    api<NginxSite>(`/api/sites/${id}`)
      .then(apply)
      .catch((e: ApiError) => setLoadError({ message: e.message, status: e.status }))
      .finally(() => setLoading(false));
  }, [id, isNew, apply]);
  useEffect(load, [load]);

  const set = <K extends keyof NginxForm>(k: K, v: NginxForm[K]) => setForm((f) => ({ ...f, [k]: v }));
  const body: NginxForm = useMemo(() => ({ ...form, aliases: splitAliases(aliasText) }), [form, aliasText]);

  // Live preview of the config this form would write (the server generates it, so it is the real text).
  const req = useRef(0);
  useEffect(() => {
    if (loading || loadError) return;
    const n = ++req.current;
    const t = setTimeout(() => {
      api<{ config: string; sslActive: boolean }>("/api/sites/preview", { method: "POST", body: { ...body, id: site?.id } })
        .then((r) => n === req.current && setPreview({ config: r.config, error: null, sslActive: r.sslActive }))
        .catch((e: ApiError) => n === req.current && setPreview((p) => ({ ...p, config: null, error: e.message })));
    }, 300);
    return () => clearTimeout(t);
  }, [body, site?.id, loading, loadError]);

  const checkDns = useCallback(() => {
    const names = [form.domain, ...splitAliases(aliasText)].filter(Boolean);
    if (!form.domain.trim()) return setDns(null);
    setDnsBusy(true);
    api<NginxDns>(`/api/sites/dns?names=${encodeURIComponent(names.join(","))}`)
      .then(setDns)
      .catch(() => setDns(null))
      .finally(() => setDnsBusy(false));
  }, [form.domain, aliasText]);
  const firstDns = useRef(false);
  useEffect(() => {
    if (site && !firstDns.current) {
      firstDns.current = true;
      checkDns();
    }
  }, [site, checkDns]);

  async function startSsl(siteId: number, force = false) {
    try {
      await api(`/api/sites/${siteId}/ssl${force ? "?force=true" : ""}`, { method: "POST" });
      if (String(siteId) !== id) navigate(`/nginx/${siteId}?ssl=1`);
      setSslRun(true);
      return true;
    } catch (e) {
      setSaveError((e as ApiError).message);
      return false;
    }
  }

  async function save() {
    setSaving(true);
    setSaveError(null);
    setFailure(null);
    try {
      const r = await api<{ site: NginxSite; needsCert: boolean; warning?: string | null }>(isNew ? "/api/sites" : `/api/sites/${id}`, {
        method: isNew ? "POST" : "PUT",
        body,
      });
      // Static folder: the server grants nginx (www-data) read access with ACLs; a problem comes back here.
      if (r.warning) toast({ title: "Saved, but nginx can't read the folder", body: r.warning, tone: "error" });
      else toast({ title: "Site saved", body: "nginx -t passed and nginx was reloaded.", tone: "success" });
      if (r.needsCert && body.sslEmail) {
        apply(r.site);
        if (await startSsl(r.site.id)) return;
      } else if (r.needsCert) {
        setSaveError("Saved without SSL: add an expiry email, then save again to request the certificate.");
      }
      if (isNew) navigate(`/nginx/${r.site.id}`);
      else apply(r.site);
      checkDns();
    } catch (e) {
      const err = e as ApiError;
      const ng = err.body?.nginx as { output?: string; line?: number | null } | undefined;
      if (ng) setFailure({ message: err.message, output: ng.output ?? "", line: ng.line ?? null });
      else setSaveError(err.message);
    } finally {
      setSaving(false);
    }
  }

  async function setEnabled(enabled: boolean) {
    if (!site) return;
    setToggling(true);
    setSaveError(null);
    setFailure(null);
    try {
      const r = await api<{ site: NginxSite }>(`/api/sites/${site.id}/${enabled ? "enable" : "disable"}`, { method: "POST" });
      apply(r.site);
      toast({ title: enabled ? "Site enabled" : "Site disabled", body: `${r.site.domain} · nginx reloaded.`, tone: "success" });
    } catch (e) {
      const err = e as ApiError;
      const ng = err.body?.nginx as { output?: string; line?: number | null } | undefined;
      if (ng) setFailure({ message: err.message, output: ng.output ?? "", line: ng.line ?? null });
      else setSaveError(err.message);
    } finally {
      setToggling(false);
    }
  }

  async function remove() {
    if (!site) return;
    setDeleting(true);
    try {
      const r = await api<{ certError: string | null }>(`/api/sites/${site.id}?deleteCert=${deleteCert}`, { method: "DELETE" });
      toast({
        title: "Site deleted",
        body: r.certError ? `${site.domain} is gone, but the certificate was kept: ${r.certError}` : `${site.domain} was removed from nginx.`,
        tone: r.certError ? "info" : "success",
      });
      navigate("/nginx");
    } catch (e) {
      setConfirmDelete(false);
      const err = e as ApiError;
      const ng = err.body?.nginx as { output?: string } | undefined;
      if (ng) setFailure({ message: err.message, output: ng.output ?? "", line: null });
      else setSaveError(err.message);
    } finally {
      setDeleting(false);
    }
  }

  if (loading) return <SkeletonBlock label="Loading site" />;
  if (loadError)
    return loadError.status === 404 ? (
      <EmptyState title="Site not found" description="It may have been deleted." action={<Button variant="secondary" href="#/nginx">Back to web server</Button>} />
    ) : (
      <ErrorState title="Could not load this site" description={loadError.message} onRetry={load} />
    );

  const customStart = preview.config ? preview.config.split("\n").findIndex((l) => l.trim() === "# custom") + 1 : 0;
  const customFail = failure?.line != null && customStart > 0 && failure.line > customStart;
  const failMsg = failure ? emergMessage(failure.output) : null;
  const app = apps.find((a) => a.id === form.appId);
  const domainDns = dns?.results.find((r) => r.name === form.domain.trim().toLowerCase());
  const aliasDns = dns?.results.filter((r) => r !== domainDns) ?? [];
  const dnsWrong = dns?.results.filter((r) => !r.ok) ?? [];
  const label = site ? sslLabel({ ...site, ssl: form.ssl }) : form.ssl ? { tone: "info" as const, text: "Not issued yet" } : { tone: "neutral" as const, text: "No SSL" };
  const issuing = sslRun;

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title={
          <span className="inline-flex flex-wrap items-center gap-3">
            <span className="font-mono">{isNew ? "New site" : site?.domain}</span>
            {site && <Badge tone={site.enabled ? "success" : "neutral"}>{site.enabled ? "Enabled" : "Disabled"}</Badge>}
          </span>
        }
        subtitle={
          site ? (
            <>
              <span className="font-mono">{site.file}</span> · last saved {formatWib(site.updatedAt)} WIB
            </>
          ) : (
            "The manager writes the config, runs nginx -t, and only then reloads nginx."
          )
        }
        actions={
          site && (
            <Button variant="subtle" href={`${site.certReady && site.ssl ? "https" : "http"}://${site.domain}`} target="_blank" rel="noreferrer">
              Open site
            </Button>
          )
        }
      />

      {failure && (
        <InlineAlert title="nginx -t failed, nothing was reloaded.">
          <p className="m-0">
            {isNew ? "The site was not created." : "The live site still uses the previous config."}{" "}
            {failure.line != null ? (customFail ? `Fix the custom directive on line ${failure.line} and save again.` : `Fix line ${failure.line} of the config and save again.`) : "See the nginx output below."}
          </p>
          {failure.output && <pre className="mt-2 whitespace-pre-wrap rounded-lg bg-surface p-3 font-mono text-[12.5px] leading-5">{failure.output}</pre>}
        </InlineAlert>
      )}
      {saveError && <InlineAlert title="Could not save">{saveError}</InlineAlert>}

      {issuing && site && (
        <NginxSslProgress
          siteId={site.id}
          onDone={(ok) => {
            api<NginxSite>(`/api/sites/${site.id}`).then(apply).catch(() => {});
            toast({ title: ok ? "SSL certificate issued" : "SSL certificate not issued", body: ok ? "nginx now serves HTTPS." : "See the log for the reason.", tone: ok ? "success" : "error" });
          }}
          onDismiss={() => {
            setSslRun(false);
            if (parseRoute(route).query.get("ssl")) navigate(`/nginx/${site.id}`);
          }}
        />
      )}

      <div className="flex flex-wrap items-start gap-6">
        <section className="min-w-0 grow-[3] basis-[560px] overflow-hidden rounded-xl border border-line bg-surface">
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (!saving && !issuing) void save();
            }}
          >
            <div className="flex flex-col gap-5 p-6">
              <h2 className="text-17 font-semibold text-ink">Domain</h2>
              <div className="grid grid-cols-[repeat(auto-fit,minmax(240px,1fr))] gap-4">
                <TextField
                  label="Domain"
                  mono
                  value={form.domain}
                  onChange={(e) => set("domain", e.target.value)}
                  onBlur={checkDns}
                  placeholder="app.example.com"
                  autoComplete="off"
                  spellCheck={false}
                  helper={
                    domainDns ? (
                      <span className={`flex items-center gap-1.5 ${domainDns.ok ? "text-success" : "text-warning"}`}>
                        {domainDns.ok ? <CheckIcon size={14} strokeWidth={2.5} /> : <AlertTriangleIcon size={14} />}
                        {domainDns.ok ? `A record → ${domainDns.ips.join(", ")}` : `${domainDns.error} Expected ${dns?.serverIp}.`}
                      </span>
                    ) : dnsBusy ? (
                      "Checking DNS…"
                    ) : (
                      `Lowercase, like app.example.com. Its A record should point to ${dns?.serverIp ?? "this server"}.`
                    )
                  }
                />
                <TextField
                  label="Aliases"
                  optional
                  mono
                  value={aliasText}
                  onChange={(e) => setAliasText(e.target.value)}
                  onBlur={checkDns}
                  placeholder="www.example.com"
                  autoComplete="off"
                  spellCheck={false}
                  helper={
                    aliasDns.length ? (
                      <span className="flex flex-col gap-0.5">
                        {aliasDns.map((r) => (
                          <span key={r.name} className={r.ok ? "text-success" : "text-warning"}>
                            {r.name}: {r.ok ? `A record → ${r.ips.join(", ")}` : r.error}
                          </span>
                        ))}
                      </span>
                    ) : (
                      "Comma separated. Each needs its own A record."
                    )
                  }
                />
              </div>
              <div>
                <Button variant="secondary" busy={dnsBusy} disabled={!form.domain.trim()} onClick={checkDns}>
                  {dnsBusy ? "Checking…" : "Check DNS again"}
                </Button>
              </div>

              <hr className="m-0 border-0 border-t border-line-soft" />
              <h2 className="text-17 font-semibold text-ink">Serves</h2>
              <div role="radiogroup" aria-label="Site type" className="grid grid-cols-[repeat(auto-fit,minmax(170px,1fr))] gap-2">
                {KINDS.map((k) => {
                  const on = form.kind === k.value;
                  return (
                    <label
                      key={k.value}
                      className={`relative flex min-h-16 cursor-pointer flex-col gap-0.5 rounded-[10px] has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-info ${
                        on ? "border-2 border-ink bg-sunken p-[11px]" : "border border-line-strong bg-surface p-3"
                      }`}
                    >
                      <input type="radio" name="kind" value={k.value} checked={on} onChange={() => set("kind", k.value)} className="absolute h-px w-px opacity-0" />
                      <span className="text-sm font-semibold text-ink">{k.label}</span>
                      <span className="text-xs text-ink-2">{k.hint}</span>
                    </label>
                  );
                })}
              </div>

              <div className="grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-4">
                {form.kind === "proxy" && (
                  <>
                    <Select
                      label="App"
                      placeholder="Select an app"
                      value={form.appId ?? ""}
                      onChange={(e) => set("appId", e.target.value ? Number(e.target.value) : null)}
                      options={apps.map((a) => ({ value: a.id, label: `${a.name}${a.port ? ` · :${a.port}` : " · no port"}` }))}
                    />
                    <TextField
                      label="Upstream"
                      mono
                      readOnly
                      value={app?.port ? `http://127.0.0.1:${app.port}` : ""}
                      placeholder="Pick an app"
                      helper={app && !app.port ? "This app has no port yet. Set one in the app's settings." : "Follows the app's port."}
                      className="bg-paper text-ink-2"
                    />
                  </>
                )}
                {form.kind === "static" && (
                  <TextField
                    label="Folder"
                    mono
                    wrapperClassName="sm:col-span-2"
                    value={form.rootPath}
                    onChange={(e) => set("rootPath", e.target.value)}
                    placeholder="/home/ubuntu/apps/landing/current/dist"
                    spellCheck={false}
                    helper="Must be inside the apps folder. nginx runs as www-data and needs read access to it."
                  />
                )}
                {form.kind === "redirect" && (
                  <TextField
                    label="Redirect to"
                    mono
                    wrapperClassName="sm:col-span-2"
                    value={form.redirectUrl}
                    onChange={(e) => set("redirectUrl", e.target.value)}
                    placeholder="https://new.example.com"
                    spellCheck={false}
                    helper="301 redirect. A URL without a path keeps the path of the request."
                  />
                )}
                {form.kind !== "redirect" && (
                  <TextField
                    label="Max upload (MB)"
                    mono
                    type="number"
                    min={1}
                    max={10240}
                    value={form.maxUploadMb}
                    onChange={(e) => set("maxUploadMb", Number(e.target.value))}
                  />
                )}
              </div>

              {form.kind !== "redirect" && (
                <div className="flex flex-wrap gap-x-8 gap-y-1">
                  {form.kind === "proxy" && <Toggle checked={form.websocket} onChange={(v) => set("websocket", v)} label="WebSocket upgrade" />}
                  <Toggle checked={form.gzip} onChange={(v) => set("gzip", v)} label="gzip" />
                  <Toggle checked={form.cacheAssets} onChange={(v) => set("cacheAssets", v)} label="Cache /assets for 7 days" />
                </div>
              )}

              <Textarea
                label="Custom directives"
                optional
                rows={6}
                value={form.customDirectives}
                onChange={(e) => set("customDirectives", e.target.value)}
                spellCheck={false}
                placeholder={"location /docs {\n    proxy_buffering off;\n}"}
                className="whitespace-pre font-mono text-13 leading-5"
                error={customFail ? (failure?.line != null ? `Line ${failure.line}: ${failMsg ?? "rejected by nginx"}.` : true) : undefined}
                helper="Free text, written inside server { }. Braces must balance; nginx -t checks the rest."
              />

              <hr className="m-0 border-0 border-t border-line-soft" />
              <div className="flex items-center justify-between gap-3">
                <h2 className="text-17 font-semibold text-ink">SSL</h2>
                <Badge tone={label.tone}>{label.text}</Badge>
              </div>
              <div className="grid grid-cols-[repeat(auto-fit,minmax(240px,1fr))] items-start gap-4">
                <div className="flex flex-col">
                  <Toggle checked={form.ssl} onChange={(v) => set("ssl", v)} label="Let’s Encrypt certificate" />
                  <Toggle checked={form.forceHttps} onChange={(v) => set("forceHttps", v)} label="Redirect HTTP to HTTPS" disabled={!form.ssl} />
                </div>
                <TextField
                  label="Expiry email"
                  type="email"
                  value={form.sslEmail}
                  onChange={(e) => set("sslEmail", e.target.value)}
                  autoComplete="email"
                  placeholder="you@example.com"
                  helper="Let’s Encrypt writes here before a certificate expires. certbot renews it by itself 30 days before."
                />
              </div>
              {form.ssl && dnsWrong.length > 0 && (
                <p className="m-0 text-13 text-warning" role="status">
                  A certificate can only be issued when {dnsWrong.map((r) => r.name).join(", ")} point{dnsWrong.length === 1 ? "s" : ""} to {dns?.serverIp}.
                </p>
              )}
              {form.ssl && !preview.sslActive && (
                <p className="m-0 text-13 text-ink-2">The config switches to HTTPS after the certificate is issued. Saving requests it right away.</p>
              )}
              {site?.ssl && !issuing && (
                <div>
                  <Button
                    variant="secondary"
                    disabled={!site.sslEmail || dnsWrong.length > 0}
                    onClick={() => startSsl(site.id, site.certReady)}
                  >
                    {site.certReady ? "Renew certificate now" : "Issue certificate"}
                  </Button>
                </div>
              )}
            </div>

            <div className="flex flex-wrap justify-between gap-2 border-t border-line bg-sunken px-6 py-4">
              <div className="flex flex-wrap gap-2">
                {site && (
                  <Button variant="secondary" busy={toggling} disabled={saving || issuing} onClick={() => setEnabled(!site.enabled)}>
                    {toggling ? (site.enabled ? "Disabling…" : "Enabling…") : site.enabled ? "Disable site" : "Enable site"}
                  </Button>
                )}
                {site && (
                  <Button variant="danger" disabled={saving || issuing} onClick={() => setConfirmDelete(true)}>
                    Delete site…
                  </Button>
                )}
              </div>
              <div className="flex flex-wrap gap-2">
                <Button variant="secondary" href="#/nginx">
                  Cancel
                </Button>
                <Button type="submit" busy={saving || issuing} disabled={!form.domain.trim()}>
                  {issuing ? "Issuing SSL…" : saving ? "Testing…" : isNew ? "Test & create" : "Test & save"}
                </Button>
              </div>
            </div>
          </form>
        </section>

        <NginxConfigPreview config={preview.config} error={form.domain.trim() ? preview.error : "Enter a domain to see the config nginx will get."} failLine={failure?.line} />
      </div>

      <ConfirmDialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        onConfirm={remove}
        busy={deleting}
        title={`Delete ${site?.domain}?`}
        confirmLabel="Delete site"
        confirmText={site?.domain}
      >
        <p className="m-0">
          This removes the {site ? KIND_LABEL[site.kind].toLowerCase() : ""} config of <strong>{site?.domain}</strong> from nginx, tests it and reloads. The domain then
          shows the default page. The app and its files are not touched.
        </p>
        {site?.certReady && (
          <div className="mt-2">
            <Checkbox checked={deleteCert} onChange={(e) => setDeleteCert(e.target.checked)} label="Also delete its Let’s Encrypt certificate" />
          </div>
        )}
      </ConfirmDialog>
    </div>
  );
}
