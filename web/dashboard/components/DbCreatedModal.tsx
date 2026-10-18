import { useState } from "react";
import { api } from "../lib/api";
import { Banner } from "./Feedback";
import { Button } from "./Button";
import { CheckIcon, CopyIcon } from "./icons";
import { Modal } from "./Modal";
import { useToast } from "./Toast";
import { copyText, type DbCreated } from "./DbApi";

/** Databases.dc.html "created": the only time the password is on screen. */
export function DbCreatedModal({ created, onDone }: { created: DbCreated | null; onDone: () => void }) {
  const toast = useToast();
  const [nag, setNag] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  if (!created) return null;
  const c = created;

  const rows: [string, string][] = [
    ["Host", c.host],
    ["Port", String(c.port)],
    ["User", c.owner],
    ["Password", c.password],
    ["URL", c.url],
  ];

  async function copy(k: string, v: string) {
    if (await copyText(v)) {
      setCopied(k);
      setNag(false);
      setTimeout(() => setCopied((cur) => (cur === k ? null : cur)), 2000);
    } else toast({ title: "Could not copy", body: "Select the value and copy it by hand.", tone: "error" });
  }

  async function restart() {
    if (!c.linkedApp) return;
    setRestarting(true);
    try {
      await api(`/api/apps/${c.linkedApp}/restart`, { method: "POST" });
      toast({ title: `Restarting ${c.linkedApp}`, tone: "success" });
    } catch (e) {
      toast({ title: `Could not restart ${c.linkedApp}`, body: (e as Error).message, tone: "error" });
    } finally {
      setRestarting(false);
    }
  }

  return (
    <Modal
      open
      // Esc, backdrop and X must not lose the password by accident: point at the button instead.
      onClose={() => setNag(true)}
      title={`Database ${c.name} is ready`}
      footer={
        <>
          {c.linkedApp && (
            <Button variant="secondary" onClick={restart} busy={restarting}>
              {restarting ? "Restarting…" : `Restart ${c.linkedApp}`}
            </Button>
          )}
          <Button onClick={onDone}>I copied it</Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <Banner tone="warning" title={nag ? "Copy the password first. " : "Copy the password now. "}>
          It is not stored here and won’t be shown again.
          {c.linkedApp ? ` It is already in ${c.linkedApp}’s .env.` : ""}
        </Banner>
        {c.warnings.map((w) => (
          <Banner key={w} tone="info">
            {w}
          </Banner>
        ))}
        <dl className="m-0 flex flex-col gap-2">
          {rows.map(([k, v]) => (
            <div key={k} className="flex items-center gap-3">
              <dt className="w-[76px] flex-none text-13 text-ink-2">{k}</dt>
              <dd className="m-0 flex h-11 min-w-0 flex-1 items-center rounded-lg border border-line bg-sunken px-3 font-mono text-13">
                <span className="min-w-0 flex-1 truncate" title={v}>
                  {v}
                </span>
              </dd>
              <Button variant="subtle" icon={copied === k ? <CheckIcon size={16} /> : <CopyIcon size={16} />} onClick={() => copy(k, v)} aria-label={`Copy ${k}`}>
                {copied === k ? "Copied" : "Copy"}
              </Button>
            </div>
          ))}
        </dl>
      </div>
    </Modal>
  );
}
