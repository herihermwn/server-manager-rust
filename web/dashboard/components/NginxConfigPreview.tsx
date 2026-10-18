import { Button } from "./Button";
import { useToast } from "./Toast";

/**
 * "Generated config" panel (NginxSite.dc.html): line numbers, and the line nginx -t rejected
 * highlighted (red ground plus an sr-only note, never color alone).
 */
export function NginxConfigPreview({ config, failLine, error }: { config: string | null; failLine?: number | null; error?: string | null }) {
  const toast = useToast();
  const lines = config ? config.replace(/\n$/, "").split("\n") : [];
  return (
    <aside className="min-w-0 grow-[2] basis-[380px] overflow-hidden rounded-xl border border-line bg-surface">
      <div className="flex items-center justify-between gap-3 border-b border-line py-3 pl-5 pr-3">
        <h2 className="text-15 font-semibold text-ink">Generated config</h2>
        <Button
          variant="subtle"
          disabled={!config}
          onClick={() => {
            navigator.clipboard?.writeText(config ?? "");
            toast({ title: "Config copied", tone: "success" });
          }}
        >
          Copy
        </Button>
      </div>
      {error ? (
        <p className="m-0 bg-sunken px-5 py-4 text-13 text-ink-2">{error}</p>
      ) : !config ? (
        <p className="m-0 bg-sunken px-5 py-4 text-13 text-ink-3" role="status">
          Generating…
        </p>
      ) : (
        <pre className="m-0 overflow-x-auto bg-sunken py-3 font-mono text-xs leading-5 text-ink" aria-label="Generated nginx config">
          {lines.map((t, i) => {
            const bad = failLine === i + 1;
            return (
              <span key={i} className={`block whitespace-pre pl-2 pr-4 ${bad ? "bg-danger-bg text-danger" : ""}`} data-line={i + 1}>
                <span className={`inline-block w-7 pr-3 text-right ${bad ? "font-semibold" : "text-ink-3"}`}>{i + 1}</span>
                {bad && <span className="sr-only">Rejected by nginx: </span>}
                {t || " "}
              </span>
            );
          })}
        </pre>
      )}
    </aside>
  );
}
