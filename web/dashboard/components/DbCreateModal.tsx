import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { Button } from "./Button";
import { InlineAlert } from "./Feedback";
import { Checkbox, Select, TextField } from "./Input";
import { Modal } from "./Modal";
import { EXTENSIONS, IDENT, appNames, randomPassword, type DbCreated } from "./DbApi";

/** Databases.dc.html "create": name, owner, password (shown/regenerated here), optional app link, extensions. */
export function DbCreateModal({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: (c: DbCreated) => void }) {
  const [name, setName] = useState("");
  const [owner, setOwner] = useState("");
  const [ownerEdited, setOwnerEdited] = useState(false);
  const [password, setPassword] = useState("");
  const [show, setShow] = useState(false);
  const [apps, setApps] = useState<string[]>([]);
  const [linkApp, setLinkApp] = useState("");
  const [ext, setExt] = useState<string[]>(["pg_trgm"]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [touched, setTouched] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName("");
    setOwner("");
    setOwnerEdited(false);
    setPassword(randomPassword());
    setShow(false);
    setLinkApp("");
    setExt(["pg_trgm"]);
    setError(null);
    setTouched(false);
    // Phase 3 builds /api/apps; on 404 or an empty list the select stays hidden.
    api<unknown>("/api/apps")
      .then((b) => setApps(appNames(b)))
      .catch(() => setApps([]));
  }, [open]);

  const nameErr = touched && !IDENT.test(name) ? "Lowercase letters, digits and _, starting with a letter or _ (max 63)." : undefined;
  const ownerErr = touched && !IDENT.test(owner) ? "Lowercase letters, digits and _, starting with a letter or _ (max 63)." : undefined;
  const pwErr = touched && password.length < 8 ? "At least 8 characters." : undefined;

  async function submit() {
    setTouched(true);
    if (!IDENT.test(name) || !IDENT.test(owner) || password.length < 8) return;
    setBusy(true);
    setError(null);
    try {
      const c = await api<DbCreated>("/api/databases", {
        body: { name, owner, password, extensions: ext, linkApp: linkApp || undefined },
      });
      onCreated(c);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      open={open}
      onClose={busy ? () => {} : onClose}
      title="Create database"
      description="A new owner user is created with it. The password is shown once."
      footer={
        <>
          <Button variant="secondary" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={submit} busy={busy}>
            {busy ? "Creating…" : "Create database"}
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        {error && <InlineAlert title="Could not create the database">{error}</InlineAlert>}
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <TextField
            label="Database name"
            mono
            value={name}
            autoFocus
            autoComplete="off"
            spellCheck={false}
            error={nameErr}
            helper="Lowercase, digits and _"
            onChange={(e) => {
              setName(e.target.value);
              if (!ownerEdited) setOwner(e.target.value);
            }}
          />
          <TextField
            label="Owner user"
            mono
            value={owner}
            autoComplete="off"
            spellCheck={false}
            error={ownerErr}
            helper="New user, can log in to this database only"
            onChange={(e) => {
              setOwner(e.target.value);
              setOwnerEdited(true);
            }}
          />
        </div>
        <div className="flex items-start gap-2">
          <TextField
            wrapperClassName="flex-1"
            label="Password"
            mono
            type={show ? "text" : "password"}
            value={password}
            autoComplete="off"
            spellCheck={false}
            error={pwErr}
            helper="32 random characters. Not stored in this dashboard; it goes into the app’s .env if you link one."
            onChange={(e) => setPassword(e.target.value)}
          />
          <div className="mt-[26px] flex gap-2">
            <Button variant="secondary" onClick={() => setShow((s) => !s)} aria-pressed={show}>
              {show ? "Hide" : "Show"}
            </Button>
            <Button variant="subtle" onClick={() => setPassword(randomPassword())}>
              Regenerate
            </Button>
          </div>
        </div>
        {apps.length > 0 && (
          <Select
            label="Link to app"
            optional
            value={linkApp}
            onChange={(e) => setLinkApp(e.target.value)}
            options={apps.map((a) => ({ value: a, label: a }))}
            placeholder="Don’t link"
            helper={
              linkApp ? (
                <>
                  Writes <span className="font-mono">DATABASE_URL</span> into {linkApp}’s .env. Restart the app to use it.
                </>
              ) : (
                "Writes DATABASE_URL into the app’s .env."
              )
            }
          />
        )}
        <fieldset className="m-0 flex flex-col gap-2 border-0 p-0">
          <legend className="pb-1.5 text-13 font-semibold text-ink">Extensions</legend>
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            {EXTENSIONS.map((x) => (
              <Checkbox
                key={x}
                label={<span className="font-mono">{x}</span>}
                checked={ext.includes(x)}
                onChange={(e) => setExt((cur) => (e.target.checked ? [...cur, x] : cur.filter((y) => y !== x)))}
              />
            ))}
          </div>
        </fieldset>
        <button type="submit" className="hidden" aria-hidden="true" tabIndex={-1} />
      </form>
    </Modal>
  );
}
