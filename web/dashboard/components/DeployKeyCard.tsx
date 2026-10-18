import { Button } from "./Button";
import { Skeleton } from "./Feedback";
import { useToast } from "./Toast";
import { copy, useApi, type DeployKey } from "./appsApi";

/** Public half of `<data>/deploy_key` (created on first use). The private key never leaves the server. */
export function useDeployKey() {
  return useApi<{ data: DeployKey }>("/api/apps/deploy-key").data?.data;
}

export function CopyKeyButton({ variant = "subtle", label = "Copy deploy key" }: { variant?: "subtle" | "secondary"; label?: string }) {
  const key = useDeployKey();
  const toast = useToast();
  return (
    <Button
      variant={variant}
      disabled={!key}
      onClick={() => key && copy(key.publicKey)?.then(() => toast({ title: "Deploy key copied", body: "Paste it in GitHub › Settings › Deploy keys, read-only." }))}
    >
      {label}
    </Button>
  );
}

/** AppNew.dc.html aside "Server deploy key". */
export function DeployKeyCard() {
  const key = useDeployKey();
  return (
    <section className="flex flex-col gap-3 rounded-xl border border-line bg-surface p-5">
      <h2 className="text-15 font-semibold">Server deploy key</h2>
      <p className="text-13 leading-5 text-ink-2">GitHub needs this public key to let the server clone. Add it once per repository, read-only.</p>
      {key ? (
        <>
          <div className="break-all rounded-lg border border-line bg-sunken px-3 py-2.5 font-mono text-xs leading-[18px] text-ink">{key.publicKey}</div>
          <div className="break-all font-mono text-xs text-ink-3">
            {key.fingerprint} · {key.type}
          </div>
        </>
      ) : (
        <Skeleton className="h-14 w-full" />
      )}
      <CopyKeyButton label="Copy public key" />
    </section>
  );
}
