//! The only way this binary runs other programs.
//!
//! - `Cmd::new("git").args([...])`: fixed program + args, no shell, so names/paths can't inject.
//!   Validate every argument that came from a request first (`validate.rs`).
//! - `run_shell_user_command(script, run_as)`: `bash -lc <script>`, as the app's user when the
//!   manager runs as root. ONLY for the build / before-restart / start commands the owner typed
//!   into an app form. Never build a script from other input.
//!
//! Every child runs in its own process group (killed as a group on timeout or cancel), with stdin
//! closed and the manager's own `SM_*` env vars removed. Output lines have the given secret values
//! replaced by `••••••` before anyone sees them.

use std::{os::unix::process::ExitStatusExt, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    sync::{Notify, mpsc},
};

pub const MASK: &str = "••••••";
/// Captured output (`Cmd::output`) is cut after this many bytes per stream.
const CAPTURE_MAX: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub stream: Stream,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
    TimedOut,
    Cancelled,
}

impl Exit {
    pub fn success(self) -> bool {
        self == Exit::Code(0)
    }
}

#[derive(Debug)]
pub struct Output {
    pub exit: Exit,
    pub stdout: String,
    pub stderr: String,
}

pub struct Cmd {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    timeout: Duration,
    secrets: Vec<String>,
    cancel: Option<Arc<Notify>>,
}

/// The system user an owner-written command runs as (apps.run_as), resolved from passwd.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunAs {
    pub user: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
}

impl RunAs {
    /// PATH with the per-user toolchains (rustup, bun) first: login shells don't read ~/.bashrc.
    pub fn path(&self) -> String {
        let h = &self.home;
        format!("{h}/.bun/bin:{h}/.cargo/bin:{h}/.local/bin:/usr/local/bin:/usr/bin:/bin")
    }
}

/// `nice -n 10 bash -lc <script>` for owner-written build/start commands (rencana §2, §6).
/// `-l` loads the login profile so rustup/bun/node are on PATH.
///
/// When the manager is root and `run_as` is given, the command first drops to that user with
/// `setpriv --reuid --regid --init-groups` (+ HOME/USER/PATH of that user). setpriv execs in place,
/// so the child stays in our process group and timeout/cancel still kill the whole build.
/// (`runuser -u` would start a new session that `killpg` can't reach.)
pub fn run_shell_user_command(script: &str, run_as: Option<&RunAs>) -> Cmd {
    // SAFETY: geteuid has no preconditions.
    let root = unsafe { libc::geteuid() } == 0;
    match run_as {
        Some(u) if root => Cmd::new("setpriv")
            .args([
                format!("--reuid={}", u.uid),
                format!("--regid={}", u.gid),
                "--init-groups".into(),
                "--".into(),
                "nice".into(),
                "-n".into(),
                "10".into(),
                "bash".into(),
                "-lc".into(),
                script.into(),
            ])
            .env("HOME", &u.home)
            .env("USER", &u.user)
            .env("LOGNAME", &u.user)
            .env("PATH", u.path()),
        Some(u) => Cmd::new("nice").args(["-n", "10", "bash", "-lc", script]).env("PATH", u.path()),
        None => Cmd::new("nice").args(["-n", "10", "bash", "-lc", script]),
    }
}

impl Cmd {
    /// Default timeout 60 s; set a longer one for builds.
    pub fn new(program: &str) -> Self {
        Cmd {
            program: program.to_owned(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            timeout: Duration::from_secs(60),
            secrets: Vec::new(),
            cancel: None,
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: Into<String>>(mut self, it: I) -> Self {
        self.args.extend(it.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    /// Values to mask in every output line (env secrets, tokens, DB passwords).
    pub fn redact<I: IntoIterator<Item = S>, S: Into<String>>(mut self, secrets: I) -> Self {
        self.secrets.extend(secrets.into_iter().map(Into::into));
        self
    }

    /// `notify.notify_one()` kills the process group and returns `Exit::Cancelled`.
    pub fn cancel_on(mut self, notify: Arc<Notify>) -> Self {
        self.cancel = Some(notify);
        self
    }

    /// Human-readable form for logs ("$ git fetch origin main"). Secrets are masked.
    pub fn display(&self) -> String {
        let s = std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        redact(&s, &self.secrets)
    }

    /// Run to the end, capturing stdout/stderr (each cut at 1 MiB).
    pub async fn output(self) -> std::io::Result<Output> {
        let (tx, mut rx) = mpsc::channel::<Line>(256);
        let collect = async {
            let (mut out, mut err) = (String::new(), String::new());
            while let Some(l) = rx.recv().await {
                let buf = if l.stream == Stream::Out { &mut out } else { &mut err };
                if buf.len() < CAPTURE_MAX {
                    buf.push_str(&l.text);
                    buf.push('\n');
                }
            }
            (out, err)
        };
        let (exit, (stdout, stderr)) = tokio::join!(self.stream(tx), collect);
        Ok(Output {
            exit: exit?,
            stdout,
            stderr,
        })
    }

    /// Run to the end, sending each redacted line to `tx` as it arrives. A dropped receiver is fine:
    /// output is still drained so the child never blocks on a full pipe.
    pub async fn stream(self, tx: mpsc::Sender<Line>) -> std::io::Result<Exit> {
        let mut c = Command::new(&self.program);
        c.args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        for (k, _) in std::env::vars_os() {
            if k.to_string_lossy().starts_with("SM_") {
                c.env_remove(k);
            }
        }
        c.envs(self.env.iter().map(|(k, v)| (k, v)));
        if let Some(d) = &self.cwd {
            c.current_dir(d);
        }
        let mut child = c.spawn()?;
        let pid = child.id().map(|p| p as i32);
        let (out, err) = (child.stdout.take(), child.stderr.take());
        let cancel = self.cancel.clone();

        let finished = tokio::select! {
            r = async {
                tokio::join!(pump(out, Stream::Out, &tx, &self.secrets), pump(err, Stream::Err, &tx, &self.secrets));
                child.wait().await
            } => Ok(r?),
            _ = tokio::time::sleep(self.timeout) => Err(Exit::TimedOut),
            _ = async { match &cancel { Some(n) => n.notified().await, None => std::future::pending().await } } => Err(Exit::Cancelled),
        };
        match finished {
            Ok(status) => Ok(match status.code() {
                Some(c) => Exit::Code(c),
                None => Exit::Signal(status.signal().unwrap_or(0)),
            }),
            Err(exit) => {
                if let Some(pid) = pid {
                    kill_group(pid, libc::SIGTERM);
                    if tokio::time::timeout(Duration::from_secs(5), child.wait())
                        .await
                        .is_err()
                    {
                        kill_group(pid, libc::SIGKILL);
                        child.wait().await.ok();
                    }
                }
                Ok(exit)
            }
        }
    }
}

fn kill_group(pgid: i32, sig: i32) {
    // SAFETY: plain syscall; pgid is the child's pid, which is its own group leader (process_group(0)).
    unsafe { libc::killpg(pgid, sig) };
}

async fn pump<R: AsyncRead + Unpin>(r: Option<R>, stream: Stream, tx: &mpsc::Sender<Line>, secrets: &[String]) {
    let Some(r) = r else { return };
    let mut r = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let s = String::from_utf8_lossy(&buf);
        let s = s.trim_end_matches(['\n', '\r']);
        // `\r` = terminal overwrite (cargo/bun progress bars): keep what the terminal would show.
        let s = s.rsplit('\r').next().unwrap_or_default();
        tx.send(Line {
            stream,
            text: redact(s, secrets),
        })
        .await
        .ok();
    }
}

/// Replace every secret value in `text` with `••••••`. Longest first, so a secret that contains
/// another is fully masked. Values under 4 characters are skipped: masking "1" or "on" would
/// shred ordinary output and hides nothing worth hiding.
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut s: Vec<&str> = secrets
        .iter()
        .map(|x| x.as_str())
        .filter(|x| x.chars().count() >= 4)
        .collect();
    if s.is_empty() {
        return text.to_owned();
    }
    s.sort_by_key(|x| std::cmp::Reverse(x.len()));
    let mut out = text.to_owned();
    for secret in s {
        if out.contains(secret) {
            out = out.replace(secret, MASK);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn redaction() {
        assert_eq!(
            redact("postgres://u:hunter22@db/x", &v(&["hunter22"])),
            "postgres://u:••••••@db/x"
        );
        assert_eq!(redact("a abcd abcdef", &v(&["abcd", "abcdef"])), "a •••••• ••••••");
        assert_eq!(redact("port 1 on", &v(&["1", "on", ""])), "port 1 on");
        assert_eq!(redact("twice s3cr3t s3cr3t", &v(&["s3cr3t"])), "twice •••••• ••••••");
        assert_eq!(redact("nothing", &[]), "nothing");
    }

    #[tokio::test]
    async fn captures_exit_code_and_env() {
        let o = Cmd::new("sh")
            .args(["-c", "echo out; echo err >&2; echo $FOO; exit 3"])
            .env("FOO", "bar-secret")
            .redact(["bar-secret"])
            .output()
            .await
            .unwrap();
        assert_eq!(o.exit, Exit::Code(3));
        assert_eq!(o.stdout, "out\n••••••\n");
        assert_eq!(o.stderr, "err\n");
    }

    #[tokio::test]
    async fn no_shell_for_fixed_args() {
        let o = Cmd::new("echo").arg("$(id); rm -rf /").output().await.unwrap();
        assert_eq!(o.stdout, "$(id); rm -rf /\n");
        assert!(o.exit.success());
    }

    #[tokio::test]
    async fn manager_env_is_not_inherited() {
        // SAFETY: test-only; no other test reads this variable.
        unsafe { std::env::set_var("SM_TEST_SECRET", "x") };
        let o = run_shell_user_command("echo \"[${SM_TEST_SECRET:-}]\"", None)
            .output()
            .await
            .unwrap();
        assert_eq!(o.stdout, "[]\n");
    }

    #[tokio::test]
    async fn timeout_kills_the_whole_group() {
        let start = std::time::Instant::now();
        // The background sleep keeps stdout open; only a group kill ends it.
        let o = run_shell_user_command("sleep 30 & sleep 30", None)
            .timeout(Duration::from_millis(300))
            .output()
            .await
            .unwrap();
        assert_eq!(o.exit, Exit::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn cancel_and_stream_lines() {
        let n = Arc::new(Notify::new());
        let (tx, mut rx) = mpsc::channel(8);
        let cmd = Cmd::new("sh")
            .args(["-c", "printf 'one\\rtwo\\n'; sleep 30"])
            .cancel_on(n.clone());
        let run = tokio::spawn(cmd.stream(tx));
        assert_eq!(
            rx.recv().await,
            Some(Line {
                stream: Stream::Out,
                text: "two".into()
            })
        );
        n.notify_one();
        assert_eq!(run.await.unwrap().unwrap(), Exit::Cancelled);
    }

    #[test]
    fn display_masks() {
        assert_eq!(
            Cmd::new("psql")
                .args(["-c", "ALTER ROLE x PASSWORD 'pw-1234'"])
                .redact(["pw-1234"])
                .display(),
            "psql -c ALTER ROLE x PASSWORD '••••••'"
        );
    }
}
