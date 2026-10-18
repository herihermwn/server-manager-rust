import { useState, type FormEvent } from "react";
import { useAuth, type LoginResult } from "../../lib/auth";
import { SERVER } from "../../lib/server";
import { Button } from "../../components/Button";
import { PasswordField, TextField } from "../../components/Input";
import { InlineAlert } from "../../components/Feedback";
import { BrandTile } from "../../components/icons";

type Failure = Extract<LoginResult, { ok: false }>;

/** Login.dc.html (default) and LoginError.dc.html (wrong password, N tries left / locked). */
function failureText(f: Failure) {
  if (f.retryAfter) return { title: "Sign-in is locked.", body: f.error };
  if (f.triesLeft !== undefined) {
    const n = f.triesLeft;
    return { title: "Wrong username or password.", body: `${n} ${n === 1 ? "try" : "tries"} left before sign-in locks for 15 minutes.` };
  }
  return { title: f.error, body: undefined };
}

export function Login() {
  const { login } = useAuth();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [failure, setFailure] = useState<Failure | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    const r = await login(username, password);
    setBusy(false);
    if (r.ok) return;
    setFailure(r);
    setPassword("");
  };

  const msg = failure && failureText(failure);

  return (
    <div className="flex min-h-screen flex-wrap bg-paper text-ink">
      <aside className="flex min-w-0 flex-[1_1_420px] flex-col justify-between gap-10 bg-console px-6 py-10 text-console-text sm:p-12">
        <div className="flex items-center gap-3">
          <BrandTile size={40} />
          <span className="text-17 font-semibold text-white">Server Manager</span>
        </div>
        <div className="flex max-w-[460px] flex-col gap-4">
          <h1 className="text-32 font-semibold text-white">Deploy, configure and watch one server from one place.</h1>
          <p className="text-15 leading-[23px] text-console-dim">Clone from git, set the folder, .env, database and nginx site, then ship with one click.</p>
        </div>
        <div className="flex flex-col gap-2 font-mono text-13 leading-5 text-console-muted">
          <span className="inline-flex items-center gap-2 text-console-text">
            <span className="h-2 w-2 rounded-full bg-term-ok" aria-hidden="true" />
            {SERVER.host}
          </span>
        </div>
      </aside>

      <main className="flex min-w-0 flex-[1_1_480px] items-center justify-center px-6 py-12">
        <form onSubmit={submit} className="flex w-full max-w-[400px] flex-col gap-5">
          <div className="flex flex-col gap-1.5">
            <h2 className="text-24 font-semibold">Sign in</h2>
            <p className="text-sm text-ink-2">Owner account for this server.</p>
          </div>

          {msg && <InlineAlert title={msg.title}>{msg.body}</InlineAlert>}

          <TextField
            label="Username"
            autoComplete="username"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            disabled={busy}
            required
            autoFocus
            className="text-15"
          />
          <PasswordField
            label="Password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            disabled={busy}
            error={!!failure}
            required
            className="text-15"
          />

          <Button type="submit" size="lg" busy={busy} className="w-full">
            {busy ? "Signing in…" : "Sign in"}
          </Button>
          <p className="text-13 leading-5 text-ink-3">
            Forgot it? Reset from the server: <span className="font-mono text-ink">server-manager create-admin &lt;username&gt;</span>
          </p>
        </form>
      </main>
    </div>
  );
}
