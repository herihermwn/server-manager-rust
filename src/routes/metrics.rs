//! Phase 2 · Usage & overview.
//!
//! - Sampler: every 10 s one row in `metrics_10s` (kept 24 h); every 5 min the finished bucket is
//!   averaged into `metrics_5m` (kept 30 days). Cache, core count and interface name ride in `extra`.
//! - API (relative to /api, behind auth): `/metrics/{current,series,processes,disk-usage,alerts}`
//!   and `/services`. Plain JSON bodies, camelCase, like `/activity`.
//! - `du` runs in a background task every 10 min and is cached; no request ever runs it.
//!
//! Alert thresholds come from the `settings` table (keys below, phase 7 edits them), else defaults:
//! `alert_disk_warn` 85, `alert_disk_stop` 95, `alert_cpu_pct` 80, `alert_cpu_minutes` 5, `alert_mem_warn` 90.

use crate::{
    exec::Cmd,
    http::{ApiResult, AppError, now},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Mutex as StdMutex, OnceLock},
    time::{Duration, Instant},
};
use sysinfo::{MINIMUM_CPU_UPDATE_INTERVAL, Networks, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::sync::Mutex;

const SAMPLE_SECS: u64 = 10;
const BUCKET: i64 = 300;
const KEEP_10S: i64 = 24 * 3600;
const KEEP_5M: i64 = 30 * 24 * 3600;
/// No sample for longer than this = the sampler is not running.
const OFFLINE_AFTER: i64 = 30;
/// WIB (Asia/Jakarta) has no DST: UTC+7 all year.
const WIB: i64 = 7 * 3600;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/metrics/current", get(current))
        .route("/metrics/series", get(series))
        .route("/metrics/processes", get(processes))
        .route("/metrics/disk-usage", get(disk_usage))
        .route("/metrics/alerts", get(alerts))
        .route("/services", get(services))
}

pub fn spawn(st: &AppState) {
    tokio::spawn(sampler(st.db.clone()));
    tokio::spawn(disk_task(st.cfg.apps_dir.clone()));
}

// ───────────────────────── samples: one shape for 10 s rows, 5 min rows and API points

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub ts: i64,
    pub cpu: f64,
    pub cpu_max: f64,
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub mem_used: f64,
    pub mem_cache: f64,
    pub mem_total: f64,
    pub swap_used: f64,
    pub swap_total: f64,
    pub disk_used: f64,
    pub disk_total: f64,
    pub disk_read: f64,
    pub disk_write: f64,
    pub net_rx: f64,
    pub net_tx: f64,
}

impl Sample {
    /// Short numbers for the wire: bytes whole, percentages and load to 2 decimals.
    fn rounded(mut self) -> Self {
        let r2 = |v: f64| (v * 100.0).round() / 100.0;
        (self.cpu, self.cpu_max, self.load1, self.load5, self.load15) = (
            r2(self.cpu),
            r2(self.cpu_max),
            r2(self.load1),
            r2(self.load5),
            r2(self.load15),
        );
        for v in [
            &mut self.mem_used,
            &mut self.mem_cache,
            &mut self.mem_total,
            &mut self.swap_used,
            &mut self.swap_total,
            &mut self.disk_used,
            &mut self.disk_total,
            &mut self.disk_read,
            &mut self.disk_write,
            &mut self.net_rx,
            &mut self.net_tx,
        ] {
            *v = v.round();
        }
        self
    }
}

/// Mean of every field; `cpu_max` is the max. None for an empty slice.
pub fn average(ts: i64, rows: &[Sample]) -> Option<Sample> {
    if rows.is_empty() {
        return None;
    }
    let n = rows.len() as f64;
    let mut o = Sample {
        ts,
        ..Default::default()
    };
    macro_rules! mean {
        ($($f:ident),*) => { $( o.$f = rows.iter().map(|r| r.$f).sum::<f64>() / n; )* };
    }
    mean!(
        cpu, load1, load5, load15, mem_used, mem_cache, mem_total, swap_used, swap_total, disk_used, disk_total,
        disk_read, disk_write, net_rx, net_tx
    );
    o.cpu_max = rows.iter().map(|r| r.cpu_max).fold(0.0, f64::max);
    Some(o)
}

/// Fold rows into `slots` equal slots of `step` seconds starting at `from`. Empty slots are
/// omitted (the chart draws a gap there); rows outside the grid are ignored.
pub fn downsample(rows: &[Sample], from: i64, step: i64, slots: i64) -> Vec<Sample> {
    let mut by_slot: BTreeMap<i64, Vec<Sample>> = BTreeMap::new();
    for r in rows {
        let i = (r.ts - from).div_euclid(step);
        if (0..slots).contains(&i) {
            by_slot.entry(i).or_default().push(r.clone());
        }
    }
    by_slot
        .into_iter()
        .filter_map(|(i, v)| average(from + i * step, &v))
        .collect()
}

/// (range secs, slot secs, from 5 min rows?). 1 h and 24 h come from the 10 s rows, ≤ 360 points each.
fn range_spec(r: &str) -> Option<(i64, i64, bool)> {
    Some(match r {
        "1h" => (3600, 10, false),
        "24h" => (86_400, 240, false),
        "7d" => (7 * 86_400, 1800, true),
        "30d" => (30 * 86_400, 7200, true),
        _ => return None,
    })
}

/// Slot grid ending with the slot that contains `now`, so the newest bar is never an empty tail.
fn grid(now: i64, range: i64, step: i64) -> (i64, i64) {
    let slots = range / step;
    (now.div_euclid(step) * step - (slots - 1) * step, slots)
}

fn wib_midnight(now: i64) -> i64 {
    (now + WIB).div_euclid(86_400) * 86_400 - WIB
}

#[derive(sqlx::FromRow)]
struct Row {
    ts: i64,
    cpu_pct: f64,
    cpu_max: f64,
    load1: f64,
    load5: f64,
    load15: f64,
    mem_used: i64,
    mem_total: i64,
    swap_used: i64,
    swap_total: i64,
    disk_used: i64,
    disk_total: i64,
    disk_read: i64,
    disk_write: i64,
    net_rx: i64,
    net_tx: i64,
    extra: String,
}

const COLS_10S: &str = "ts, cpu_pct, cpu_pct AS cpu_max, load1, load5, load15, mem_used, mem_total, swap_used, swap_total, disk_used, disk_total, disk_read, disk_write, net_rx, net_tx, extra";
const COLS_5M: &str = "ts, cpu_pct, cpu_max, load1, load5, load15, mem_used, mem_total, swap_used, swap_total, disk_used, disk_total, disk_read, disk_write, net_rx, net_tx, extra";

impl Row {
    fn extra(&self) -> Value {
        serde_json::from_str(&self.extra).unwrap_or_default()
    }
    fn sample(&self) -> Sample {
        Sample {
            ts: self.ts,
            cpu: self.cpu_pct,
            cpu_max: self.cpu_max,
            load1: self.load1,
            load5: self.load5,
            load15: self.load15,
            mem_used: self.mem_used as f64,
            mem_cache: self.extra()["memCache"].as_f64().unwrap_or(0.0),
            mem_total: self.mem_total as f64,
            swap_used: self.swap_used as f64,
            swap_total: self.swap_total as f64,
            disk_used: self.disk_used as f64,
            disk_total: self.disk_total as f64,
            disk_read: self.disk_read as f64,
            disk_write: self.disk_write as f64,
            net_rx: self.net_rx as f64,
            net_tx: self.net_tx as f64,
        }
    }
}

async fn rows_between(db: &sqlx::SqlitePool, five_min: bool, from: i64, to: i64) -> Result<Vec<Row>, sqlx::Error> {
    let (cols, table) = if five_min {
        (COLS_5M, "metrics_5m")
    } else {
        (COLS_10S, "metrics_10s")
    };
    sqlx::query_as(&format!(
        "SELECT {cols} FROM {table} WHERE ts >= ? AND ts < ? ORDER BY ts"
    ))
    .bind(from)
    .bind(to)
    .fetch_all(db)
    .await
}

// ───────────────────────── sampler

struct Sampler {
    sys: System,
    nets: Networks,
    net_at: Instant,
    disk: Option<(Instant, u64, u64)>,
}

fn is_virtual_iface(n: &str) -> bool {
    n == "lo"
        || n.starts_with("lo") && n[2..].chars().all(|c| c.is_ascii_digit())
        || ["docker", "veth", "br-", "virbr", "lxc", "cni", "flannel", "tun", "tap"]
            .iter()
            .any(|p| n.starts_with(p))
}

/// Cumulative (read, write) bytes of whole physical disks from /proc/diskstats (512-byte sectors).
/// `is_whole` tells a disk from its partitions (Linux: it has an entry in /sys/block).
fn parse_diskstats(text: &str, is_whole: impl Fn(&str) -> bool) -> (u64, u64) {
    let (mut r, mut w) = (0, 0);
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some(name) = f.get(2) else { continue };
        if ["loop", "ram", "dm-", "sr", "zram", "md"]
            .iter()
            .any(|p| name.starts_with(p))
            || !is_whole(name)
        {
            continue;
        }
        r += f.get(5).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * 512;
        w += f.get(9).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * 512;
    }
    (r, w)
}

/// (used, total) bytes of `/`, the way `df` counts them.
#[allow(clippy::useless_conversion)] // statvfs field widths differ per OS
fn root_disk() -> (u64, u64) {
    // SAFETY: statvfs fills a zeroed struct for a valid NUL-terminated path.
    unsafe {
        let mut s: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c"/".as_ptr(), &mut s) != 0 {
            return (0, 0);
        }
        let unit = u64::from(s.f_frsize);
        let total = u64::from(s.f_blocks) * unit;
        (total.saturating_sub(u64::from(s.f_bfree) * unit), total)
    }
}

impl Sampler {
    fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        Sampler {
            sys,
            nets: Networks::new_with_refreshed_list(),
            net_at: Instant::now(),
            disk: None,
        }
    }

    fn sample(&mut self, ts: i64) -> (Sample, Value) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let total = self.sys.total_memory();
        let avail = self.sys.available_memory().min(total);
        let load = System::load_average();

        let secs = self.net_at.elapsed().as_secs_f64().max(0.5);
        self.nets.refresh(true);
        self.net_at = Instant::now();
        let primary = self
            .nets
            .iter()
            .filter(|(n, _)| !is_virtual_iface(n))
            .max_by_key(|(_, d)| d.total_received() + d.total_transmitted());
        let (iface, rx, tx) = primary.map_or((String::new(), 0, 0), |(n, d)| {
            (n.clone(), d.received(), d.transmitted())
        });

        let (disk_read, disk_write) = if cfg!(target_os = "linux") {
            let text = std::fs::read_to_string("/proc/diskstats").unwrap_or_default();
            let (r, w) = parse_diskstats(&text, |n| Path::new("/sys/block").join(n).exists());
            let prev = self.disk.replace((Instant::now(), r, w));
            prev.map_or((0.0, 0.0), |(t, pr, pw)| {
                let dt = t.elapsed().as_secs_f64().max(0.5);
                (r.saturating_sub(pr) as f64 / dt, w.saturating_sub(pw) as f64 / dt)
            })
        } else {
            (0.0, 0.0)
        };
        let (disk_used, disk_total) = root_disk();

        let cpu = f64::from(self.sys.global_cpu_usage());
        let s = Sample {
            ts,
            cpu,
            cpu_max: cpu,
            load1: load.one,
            load5: load.five,
            load15: load.fifteen,
            mem_used: (total - avail) as f64,
            mem_cache: avail.saturating_sub(self.sys.free_memory()) as f64,
            mem_total: total as f64,
            swap_used: self.sys.used_swap() as f64,
            swap_total: self.sys.total_swap() as f64,
            disk_used: disk_used as f64,
            disk_total: disk_total as f64,
            disk_read,
            disk_write,
            net_rx: rx as f64 / secs,
            net_tx: tx as f64 / secs,
        };
        let extra = json!({ "cores": self.sys.cpus().len(), "memCache": s.mem_cache.round(), "iface": iface });
        (s, extra)
    }
}

/// Insert a sample into `table` (`metrics_10s` or `metrics_5m`, the latter with `samples`).
async fn insert(db: &sqlx::SqlitePool, s: &Sample, extra: &Value, five_min: Option<i64>) -> Result<(), sqlx::Error> {
    let (table, extra_cols, extra_vals) = match five_min {
        Some(_) => ("metrics_5m", ", cpu_max, samples", ", ?, ?"),
        None => ("metrics_10s", "", ""),
    };
    let q = format!(
        "INSERT OR REPLACE INTO {table} (ts, cpu_pct, load1, load5, load15, mem_used, mem_total, swap_used, swap_total, disk_used, disk_total, disk_read, disk_write, net_rx, net_tx, extra{extra_cols}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?{extra_vals})"
    );
    let r = |v: f64| v.round() as i64;
    let mut q = sqlx::query(&q)
        .bind(s.ts)
        .bind(s.cpu)
        .bind(s.load1)
        .bind(s.load5)
        .bind(s.load15)
        .bind(r(s.mem_used))
        .bind(r(s.mem_total))
        .bind(r(s.swap_used))
        .bind(r(s.swap_total))
        .bind(r(s.disk_used))
        .bind(r(s.disk_total))
        .bind(r(s.disk_read))
        .bind(r(s.disk_write))
        .bind(r(s.net_rx))
        .bind(r(s.net_tx))
        .bind(extra.to_string());
    if let Some(n) = five_min {
        q = q.bind(s.cpu_max).bind(n);
    }
    q.execute(db).await.map(|_| ())
}

/// Average every finished 5 min bucket that has no `metrics_5m` row yet (also catches up after a
/// restart; the 10 s rows only reach back 24 h), then drop rows past retention.
async fn rollup_and_prune(db: &sqlx::SqlitePool, now: i64) -> Result<(), sqlx::Error> {
    let last = now.div_euclid(BUCKET) * BUCKET - BUCKET;
    let newest: Option<i64> = sqlx::query_scalar("SELECT MAX(ts) FROM metrics_5m")
        .fetch_one(db)
        .await?;
    let mut b = newest.map_or(now - KEEP_10S, |t| t + BUCKET).max(now - KEEP_10S);
    b = b.div_euclid(BUCKET) * BUCKET;
    while b <= last {
        let rows = rows_between(db, false, b, b + BUCKET).await?;
        let samples: Vec<Sample> = rows.iter().map(Row::sample).collect();
        if let (Some(avg), Some(first)) = (average(b, &samples), rows.first()) {
            let mut extra = first.extra();
            extra["memCache"] = json!(avg.mem_cache.round());
            insert(db, &avg, &extra, Some(samples.len() as i64)).await?;
        }
        b += BUCKET;
    }
    sqlx::query("DELETE FROM metrics_10s WHERE ts < ?")
        .bind(now - KEEP_10S)
        .execute(db)
        .await?;
    sqlx::query("DELETE FROM metrics_5m WHERE ts < ?")
        .bind(now - KEEP_5M)
        .execute(db)
        .await?;
    Ok(())
}

async fn sampler(db: sqlx::SqlitePool) {
    let mut s = Sampler::new();
    tokio::time::sleep(MINIMUM_CPU_UPDATE_INTERVAL * 2).await; // CPU % needs two reads
    let mut tick = tokio::time::interval(Duration::from_secs(SAMPLE_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_roll: i64 = 0;
    loop {
        tick.tick().await;
        let ts = now();
        let (sample, extra) = s.sample(ts);
        if let Err(e) = insert(&db, &sample, &extra, None).await {
            eprintln!("[metrics] insert: {e}");
        }
        if ts.div_euclid(BUCKET) != last_roll.div_euclid(BUCKET) {
            last_roll = ts;
            if let Err(e) = rollup_and_prune(&db, ts).await {
                eprintln!("[metrics] rollup: {e}");
            }
        }
    }
}

// ───────────────────────── thresholds and alerts

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thresholds {
    pub disk_warn: f64,
    pub disk_stop: f64,
    pub cpu_pct: f64,
    pub cpu_minutes: f64,
    pub mem_warn: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            disk_warn: 85.0,
            disk_stop: 95.0,
            cpu_pct: 80.0,
            cpu_minutes: 5.0,
            mem_warn: 90.0,
        }
    }
}

impl Thresholds {
    fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut t = Thresholds::default();
        for (k, v) in pairs {
            let Ok(v) = v.trim().parse::<f64>() else { continue };
            if !v.is_finite() || v <= 0.0 {
                continue;
            }
            match k.as_str() {
                "alert_disk_warn" => t.disk_warn = v,
                "alert_disk_stop" => t.disk_stop = v,
                "alert_cpu_pct" => t.cpu_pct = v,
                "alert_cpu_minutes" => t.cpu_minutes = v,
                "alert_mem_warn" => t.mem_warn = v,
                _ => {}
            }
        }
        t
    }
}

async fn thresholds(db: &sqlx::SqlitePool) -> Thresholds {
    let pairs: Vec<(String, String)> =
        sqlx::query_as("SELECT key, value FROM settings WHERE key LIKE 'alert\\_%' ESCAPE '\\'")
            .fetch_all(db)
            .await
            .unwrap_or_default();
    Thresholds::from_pairs(pairs)
}

#[derive(Debug, Serialize)]
pub struct Alert {
    pub key: String,
    /// "warning" | "critical"
    pub level: &'static str,
    pub title: String,
    pub detail: String,
    /// Topbar meta: "disk 92%".
    pub short: String,
    pub href: &'static str,
}

fn pct(used: f64, total: f64) -> f64 {
    if total > 0.0 { used / total * 100.0 } else { 0.0 }
}

/// Length in seconds of the unbroken run of samples above `limit`, newest first. A hole longer
/// than 3 samples ends the run (the sampler was down, so we can't claim it stayed high).
pub fn high_run_secs(newest_first: &[(i64, f64)], limit: f64) -> i64 {
    let Some(&(newest, _)) = newest_first.first() else {
        return 0;
    };
    let (mut oldest, mut prev) = (newest, newest);
    for &(ts, cpu) in newest_first {
        if cpu <= limit || prev - ts > 3 * SAMPLE_SECS as i64 {
            break;
        }
        oldest = ts;
        prev = ts;
    }
    if newest_first[0].1 <= limit {
        0
    } else {
        newest - oldest + SAMPLE_SECS as i64
    }
}

pub fn evaluate(t: &Thresholds, cur: Option<&Sample>, cpu_run_secs: i64, failed: &[String]) -> Vec<Alert> {
    let mut out = Vec::new();
    if let Some(c) = cur {
        let disk = pct(c.disk_used, c.disk_total).round();
        if disk >= t.disk_stop {
            out.push(Alert {
                key: "disk".into(),
                level: "critical",
                title: format!("Disk is {disk}% full."),
                detail: format!("Deploys are blocked until it drops below {}%.", t.disk_stop),
                short: format!("disk {disk}%"),
                href: "#/usage",
            });
        } else if disk >= t.disk_warn {
            out.push(Alert {
                key: "disk".into(),
                level: "warning",
                title: format!("Disk is {disk}% full."),
                detail: format!("Deploys are blocked at {}%.", t.disk_stop),
                short: format!("disk {disk}%"),
                href: "#/usage",
            });
        }
        if cpu_run_secs as f64 >= t.cpu_minutes * 60.0 {
            out.push(Alert {
                key: "cpu".into(),
                level: "warning",
                title: format!("CPU above {}% for {} min.", t.cpu_pct, cpu_run_secs / 60),
                detail: "Check which process is using it.".into(),
                short: format!("cpu {}%", c.cpu.round()),
                href: "#/usage",
            });
        }
        let mem = pct(c.mem_used, c.mem_total).round();
        if mem >= t.mem_warn {
            out.push(Alert {
                key: "memory".into(),
                level: "warning",
                title: format!("Memory is {mem}% used."),
                detail: "Apps may be killed when it runs out.".into(),
                short: format!("memory {mem}%"),
                href: "#/usage",
            });
        }
    }
    for name in failed {
        out.push(Alert {
            key: format!("service:{name}"),
            level: "critical",
            title: format!("{name} has failed."),
            detail: format!("systemd reports {name} as failed."),
            short: format!("{name} failed"),
            href: "#/",
        });
    }
    out
}

// ───────────────────────── handlers: current, series, alerts

async fn latest(db: &sqlx::SqlitePool) -> Result<Option<Row>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLS_10S} FROM metrics_10s ORDER BY ts DESC LIMIT 1"))
        .fetch_optional(db)
        .await
}

fn os_name() -> &'static str {
    static OS: OnceLock<String> = OnceLock::new();
    OS.get_or_init(|| {
        [System::name(), System::os_version()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// GET /metrics/current: newest sample plus what the KPI strips need around it.
async fn current(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let now = now();
    let Some(row) = latest(&st.db).await? else {
        return Ok(Json(json!({ "sample": null, "online": false })));
    };
    let sample = row.sample();
    let extra = row.extra();
    let peak: Option<(f64, i64)> =
        sqlx::query_as("SELECT cpu_pct, ts FROM metrics_10s WHERE ts >= ? ORDER BY cpu_pct DESC, ts DESC LIMIT 1")
            .bind(now - 3600)
            .fetch_optional(&st.db)
            .await?;
    let today: (Option<i64>, Option<i64>, Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT SUM(net_rx), SUM(net_tx), MAX(net_rx), MAX(net_tx) FROM metrics_10s WHERE ts >= ?")
            .bind(wib_midnight(now))
            .fetch_one(&st.db)
            .await?;
    let age = (now - sample.ts).max(0);
    Ok(Json(json!({
        "sample": sample.rounded(),
        "ageSecs": age,
        "online": age <= OFFLINE_AFTER,
        "cores": extra["cores"],
        "iface": extra["iface"],
        "uptimeSecs": System::uptime(),
        "os": os_name(),
        "peak": peak.map(|(cpu, ts)| json!({ "cpu": cpu, "ts": ts })),
        // Bytes today (WIB): each 10 s sample counts as 10 s of its rate.
        "today": {
            "rx": today.0.unwrap_or(0) * SAMPLE_SECS as i64,
            "tx": today.1.unwrap_or(0) * SAMPLE_SECS as i64,
            "peakRx": today.2.unwrap_or(0),
            "peakTx": today.3.unwrap_or(0),
        },
    })))
}

#[derive(Deserialize)]
struct SeriesQuery {
    range: Option<String>,
}

/// GET /metrics/series?range=1h|24h|7d|30d. Points sit on a fixed grid (`from` + n × `step`);
/// slots without a sample are left out so the chart shows a gap.
async fn series(State(st): State<AppState>, Query(q): Query<SeriesQuery>) -> ApiResult<Json<Value>> {
    let range = q.range.as_deref().unwrap_or("1h");
    let Some((secs, step, five_min)) = range_spec(range) else {
        return Err(AppError::bad_request("range must be 1h, 24h, 7d or 30d"));
    };
    let (from, slots) = grid(now(), secs, step);
    let samples: Vec<Sample> = rows_between(&st.db, five_min, from, from + slots * step)
        .await?
        .iter()
        .map(Row::sample)
        .collect();
    Ok(Json(json!({
        "range": range,
        "from": from,
        "step": step,
        "slots": slots,
        "points": downsample(&samples, from, step, slots).into_iter().map(Sample::rounded).collect::<Vec<_>>(),
    })))
}

/// GET /metrics/alerts: warnings from the thresholds, plus failed services.
async fn alerts(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let t = thresholds(&st.db).await;
    let now = now();
    let cur = latest(&st.db)
        .await?
        .map(|r| r.sample())
        .filter(|s| now - s.ts <= OFFLINE_AFTER);
    let take = (t.cpu_minutes * 6.0) as i64 + 12;
    let cpu: Vec<(i64, f64)> = sqlx::query_as("SELECT ts, cpu_pct FROM metrics_10s ORDER BY ts DESC LIMIT ?")
        .bind(take)
        .fetch_all(&st.db)
        .await?;
    let failed: Vec<String> = service_list()
        .await
        .into_iter()
        .filter(|s| s.state == "failed")
        .map(|s| s.name)
        .collect();
    let list = evaluate(&t, cur.as_ref(), high_run_secs(&cpu, t.cpu_pct), &failed);
    Ok(Json(json!({ "alerts": list, "thresholds": t })))
}

// ───────────────────────── processes

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    /// App name when this is the main process of the systemd unit `app-<name>.service`.
    pub app: Option<String>,
    /// Percent of one core, like `top` (can exceed 100).
    pub cpu: f64,
    pub mem_bytes: u64,
    pub uptime_secs: u64,
}

/// Top `n` by CPU (ties: memory), plus every app process even when idle.
fn pick_top(mut all: Vec<Proc>, n: usize) -> Vec<Proc> {
    all.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem_bytes.cmp(&a.mem_bytes)));
    all.into_iter()
        .enumerate()
        .filter(|(i, p)| *i < n || p.app.is_some())
        .map(|(_, p)| p)
        .collect()
}

/// `systemctl show` blocks (blank-line separated, `Key=value` lines) → main pid → app name.
fn parse_app_units(text: &str) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for block in text.split("\n\n") {
        let get = |k: &str| block.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('='));
        let (Some(id), Some(pid)) = (get("Id"), get("MainPID").and_then(|p| p.parse::<u32>().ok())) else {
            continue;
        };
        if let (Some(app), true) = (
            id.strip_prefix("app-").and_then(|s| s.strip_suffix(".service")),
            pid > 0,
        ) {
            out.insert(pid, app.to_owned());
        }
    }
    out
}

async fn app_pids() -> HashMap<u32, String> {
    if !cfg!(target_os = "linux") {
        return HashMap::new();
    }
    let out = Cmd::new("systemctl")
        .args(["show", "app-*.service", "-p", "Id", "-p", "MainPID"])
        .timeout(Duration::from_secs(5))
        .output()
        .await;
    out.map(|o| parse_app_units(&o.stdout)).unwrap_or_default()
}

struct ProcCache {
    sys: System,
    at: Option<Instant>,
    rows: Vec<Proc>,
}

static PROCS: Mutex<Option<ProcCache>> = Mutex::const_new(None);

fn refresh_procs(sys: &mut System) {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );
}

/// GET /metrics/processes. One `System` is kept between calls so CPU % is a real delta; a result
/// is reused for 4 s, so polling from several tabs costs one scan.
async fn processes() -> Json<Vec<Proc>> {
    let mut guard = PROCS.lock().await;
    let c = guard.get_or_insert_with(|| ProcCache {
        sys: System::new(),
        at: None,
        rows: Vec::new(),
    });
    if c.at.is_none_or(|t| t.elapsed() > Duration::from_secs(4)) {
        let cold = c.at.is_none_or(|t| t.elapsed() > Duration::from_secs(30));
        refresh_procs(&mut c.sys);
        if cold {
            tokio::time::sleep(MINIMUM_CPU_UPDATE_INTERVAL).await;
            refresh_procs(&mut c.sys);
        }
        let apps = app_pids().await;
        let now_secs = now().max(0) as u64;
        let all = c
            .sys
            .processes()
            .iter()
            .map(|(pid, p)| Proc {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                app: apps.get(&pid.as_u32()).cloned(),
                cpu: (f64::from(p.cpu_usage()) * 10.0).round() / 10.0,
                mem_bytes: p.memory(),
                uptime_secs: now_secs.saturating_sub(p.start_time()),
            })
            .collect();
        c.rows = pick_top(all, 8);
        c.at = Some(Instant::now());
    }
    Json(c.rows.clone())
}

// ───────────────────────── services

#[derive(Clone, Debug, Serialize)]
pub struct Service {
    pub name: String,
    /// `systemctl is-active` output, or "unknown" when systemctl is not there (macOS dev).
    pub state: String,
    pub version: Option<String>,
}

/// First dotted number in a `--version` line: "nginx version: nginx/1.24.0 (Ubuntu)" → 1.24.0.
fn first_version(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '/' || c == '(' || c == ')')
        .find(|t| t.contains('.') && t.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_owned)
}

async fn version_of(program: &str, arg: &str) -> Option<String> {
    let o = Cmd::new(program)
        .arg(arg)
        .timeout(Duration::from_secs(5))
        .output()
        .await
        .ok()?;
    first_version(&format!("{}\n{}", o.stdout, o.stderr))
}

const SERVICES: [&str; 4] = ["nginx", "postgresql", "certbot.timer", "server-manager"];

struct SvcCache {
    at: Instant,
    list: Vec<Service>,
}

static SVC: Mutex<Option<SvcCache>> = Mutex::const_new(None);
static VERSIONS: tokio::sync::OnceCell<[Option<String>; 4]> = tokio::sync::OnceCell::const_new();

/// Cached 5 s: the Overview, the topbar and the alerts all ask.
async fn service_list() -> Vec<Service> {
    let mut g = SVC.lock().await;
    if let Some(c) = g.as_ref().filter(|c| c.at.elapsed() < Duration::from_secs(5)) {
        return c.list.clone();
    }
    let versions = VERSIONS
        .get_or_init(|| async {
            [
                version_of("nginx", "-v").await,
                version_of("psql", "--version").await,
                version_of("certbot", "--version").await,
                Some(env!("CARGO_PKG_VERSION").to_owned()),
            ]
        })
        .await;
    // is-active exits non-zero when any unit is inactive; the lines are still one per unit.
    let out = Cmd::new("systemctl")
        .arg("is-active")
        .args(SERVICES)
        .timeout(Duration::from_secs(5))
        .output()
        .await;
    let lines: Vec<String> = out
        .map(|o| o.stdout.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    let list: Vec<Service> = SERVICES
        .iter()
        .enumerate()
        .map(|(i, name)| Service {
            name: (*name).to_owned(),
            state: lines
                .get(i)
                .filter(|_| lines.len() == SERVICES.len())
                .cloned()
                .unwrap_or_else(|| "unknown".into()),
            version: versions[i].clone(),
        })
        .collect();
    *g = Some(SvcCache {
        at: Instant::now(),
        list: list.clone(),
    });
    list
}

/// GET /services.
async fn services() -> Json<Vec<Service>> {
    Json(service_list().await)
}

// ───────────────────────── disk usage by folder (background `du`)

#[derive(Clone, Debug, Serialize)]
struct DiskEntry {
    key: &'static str,
    label: &'static str,
    bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
struct DiskSnapshot {
    at: i64,
    entries: Vec<DiskEntry>,
}

static DISK_USAGE: StdMutex<Option<DiskSnapshot>> = StdMutex::new(None);

/// Sum of `du` over `paths` (0 when there are none). Unreadable subfolders only print warnings;
/// the total line is still the last line.
async fn du_total(paths: Vec<String>) -> u64 {
    if paths.is_empty() {
        return 0;
    }
    let (flag, unit) = if cfg!(target_os = "macos") {
        ("-skc", 1024)
    } else {
        ("-sbc", 1)
    };
    let out = Cmd::new("nice")
        .args(["-n", "19", "du", flag])
        .args(paths)
        .timeout(Duration::from_secs(300))
        .output()
        .await;
    let Ok(o) = out else { return 0 };
    o.stdout
        .lines()
        .last()
        .and_then(|l| l.split_whitespace().next())
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or(0)
        * unit
}

/// `<apps>/*/<sub>/` for every app folder that has it (trailing slash follows the `current` symlink).
fn app_subdirs(apps_dir: &Path, sub: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(apps_dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .flatten()
        .map(|e| e.path().join(sub))
        .filter(|p| p.is_dir())
        .map(|p| format!("{}/", p.display()))
        .collect();
    v.sort();
    v
}

async fn measure_disk(apps_dir: &Path) -> DiskSnapshot {
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let existing = |p: PathBuf| {
        if std::fs::read_dir(&p).is_ok() {
            vec![p.display().to_string()]
        } else {
            Vec::new()
        }
    };
    let mut entries = Vec::new();
    for (key, label, paths) in [
        ("current", "~/apps/*/current", app_subdirs(apps_dir, "current")),
        ("releases", "~/apps/*/releases", app_subdirs(apps_dir, "releases")),
        ("backups", "~/backups", existing(home.join("backups"))),
        // postgres data is owned by the postgres user: skipped unless we can list it.
        (
            "postgres",
            "/var/lib/postgresql",
            existing("/var/lib/postgresql".into()),
        ),
        ("logs", "/var/log", existing("/var/log".into())),
    ] {
        if paths.is_empty() && matches!(key, "postgres" | "backups") {
            continue;
        }
        entries.push(DiskEntry {
            key,
            label,
            bytes: du_total(paths).await,
        });
    }
    DiskSnapshot { at: now(), entries }
}

async fn disk_task(apps_dir: PathBuf) {
    tokio::time::sleep(Duration::from_secs(15)).await; // not during boot
    loop {
        let snap = measure_disk(&apps_dir).await;
        *DISK_USAGE.lock().unwrap() = Some(snap);
        tokio::time::sleep(Duration::from_secs(600)).await;
    }
}

/// GET /metrics/disk-usage: cached sizes (refreshed every 10 min). `other` = used on `/` minus the listed folders.
async fn disk_usage(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let Some(snap) = DISK_USAGE.lock().unwrap().clone() else {
        return Ok(Json(json!({ "at": null, "entries": [], "otherBytes": null })));
    };
    let used = latest(&st.db).await?.map_or(0, |r| r.disk_used.max(0) as u64);
    let listed: u64 = snap.entries.iter().map(|e| e.bytes).sum();
    Ok(Json(
        json!({ "at": snap.at, "entries": snap.entries, "otherBytes": used.saturating_sub(listed) }),
    ))
}

// ───────────────────────── tests

#[cfg(test)]
mod tests {
    use super::*;

    fn s(ts: i64, cpu: f64, rx: f64) -> Sample {
        Sample {
            ts,
            cpu,
            cpu_max: cpu,
            net_rx: rx,
            mem_total: 100.0,
            ..Default::default()
        }
    }

    #[test]
    fn average_is_mean_and_cpu_max_is_max() {
        let a = average(300, &[s(300, 10.0, 100.0), s(310, 30.0, 300.0), s(320, 20.0, 200.0)]).unwrap();
        assert_eq!(
            (a.ts, a.cpu, a.cpu_max, a.net_rx, a.mem_total),
            (300, 20.0, 30.0, 200.0, 100.0)
        );
        assert!(average(0, &[]).is_none());
        // a rolled-up row keeps its own max when averaged again (7 d / 30 d charts)
        let mut r1 = s(0, 10.0, 0.0);
        r1.cpu_max = 90.0;
        assert_eq!(average(0, &[r1, s(0, 20.0, 0.0)]).unwrap().cpu_max, 90.0);
    }

    #[test]
    fn downsample_buckets_leaves_gaps_and_ignores_outside() {
        // grid: from 1000, step 240, 360 slots
        let rows: Vec<Sample> = (0..60).map(|i| s(1000 + i * 10, i as f64, 0.0)).collect(); // 600 s of data
        let d = downsample(&rows, 1000, 240, 360);
        assert_eq!(d.len(), 3); // slots 0, 1, 2; the rest are gaps
        assert_eq!(d[0].ts, 1000);
        assert_eq!(d[1].ts, 1240);
        assert_eq!(d[0].cpu, (0..24).sum::<i32>() as f64 / 24.0);
        assert_eq!(d[0].cpu_max, 23.0);
        let out = [s(999, 1.0, 0.0), s(1000 + 360 * 240, 1.0, 0.0)];
        assert!(downsample(&out, 1000, 240, 360).is_empty());
        // never more than 360 points whatever the range
        for r in ["1h", "24h", "7d", "30d"] {
            let (secs, step, _) = range_spec(r).unwrap();
            assert!(secs / step <= 360, "{r}");
        }
        assert!(range_spec("1y").is_none());
    }

    #[test]
    fn grid_ends_on_the_slot_containing_now() {
        let (from, slots) = grid(10_005, 3600, 10);
        assert_eq!((from, slots), (10_000 - 359 * 10, 360));
        assert_eq!(from + (slots - 1) * 10, 10_000);
    }

    #[test]
    fn wib_day_starts_at_17_utc() {
        // 2026-10-18 20:00 UTC = 03:00 WIB on the 19th → WIB day began 2026-10-18 17:00 UTC
        let t = 1_792_353_600; // 2026-10-18 20:00:00 UTC
        assert_eq!(wib_midnight(t), t - 3 * 3600);
        assert_eq!(wib_midnight(t - 4 * 3600), t - 3 * 3600 - 86_400);
    }

    #[test]
    fn thresholds_use_defaults_and_settings() {
        assert_eq!(Thresholds::from_pairs([]), Thresholds::default());
        let t = Thresholds::from_pairs([
            ("alert_disk_warn".into(), "70".into()),
            ("alert_cpu_minutes".into(), " 2 ".into()),
            ("alert_mem_warn".into(), "nope".into()),
            ("alert_disk_stop".into(), "-1".into()),
            ("unrelated".into(), "5".into()),
        ]);
        assert_eq!(
            (t.disk_warn, t.cpu_minutes, t.mem_warn, t.disk_stop),
            (70.0, 2.0, 90.0, 95.0)
        );
    }

    fn cur(disk: f64, mem: f64) -> Sample {
        Sample {
            ts: 1,
            cpu: 91.0,
            disk_used: disk,
            disk_total: 100.0,
            mem_used: mem,
            mem_total: 100.0,
            ..Default::default()
        }
    }

    #[test]
    fn alerts_follow_thresholds() {
        let t = Thresholds::default();
        assert!(evaluate(&t, Some(&cur(84.0, 89.0)), 299, &[]).is_empty());
        let a = evaluate(&t, Some(&cur(85.0, 90.0)), 300, &[]);
        let keys: Vec<_> = a.iter().map(|a| (a.key.as_str(), a.level)).collect();
        assert_eq!(keys, [("disk", "warning"), ("cpu", "warning"), ("memory", "warning")]);
        let a = evaluate(&t, Some(&cur(96.0, 10.0)), 0, &["postgresql".into()]);
        assert_eq!((a[0].level, a[0].short.as_str()), ("critical", "disk 96%"));
        assert_eq!((a[1].key.as_str(), a[1].level), ("service:postgresql", "critical"));
        // no fresh sample: no metric alerts, failed services still count
        assert_eq!(evaluate(&t, None, 9999, &[]).len(), 0);
        // a custom threshold wins
        let t2 = Thresholds { disk_warn: 50.0, ..t };
        assert_eq!(evaluate(&t2, Some(&cur(60.0, 0.0)), 0, &[]).len(), 1);
    }

    #[test]
    fn cpu_run_counts_back_until_a_dip_or_a_hole() {
        let hi = |ts: i64| (ts, 90.0);
        // 31 samples (300 s) above 80, then a dip
        let mut v: Vec<(i64, f64)> = (0..31).map(|i| hi(1000 - i * 10)).collect();
        assert_eq!(high_run_secs(&v, 80.0), 310);
        v.push((600, 10.0));
        assert_eq!(high_run_secs(&v, 80.0), 310);
        // newest sample is calm → 0
        assert_eq!(high_run_secs(&[(1000, 5.0), (990, 99.0)], 80.0), 0);
        // a hole (sampler was down) ends the run
        assert_eq!(high_run_secs(&[hi(1000), hi(990), hi(500), hi(490)], 80.0), 20);
        assert_eq!(high_run_secs(&[], 80.0), 0);
    }

    #[test]
    fn diskstats_sums_whole_disks_only() {
        let text = "   7       0 loop0 11 0 28 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n\
 253       0 vda 19695 4388 1308022 22766 59305 67188 7051016 160719 0 42925 186110 0 0 0 0 20966 2624\n\
 253       2 vda2 18974 4388 1297869 22223 59293 67180 7050912 160709 0 45475 182932 0 0 0 0 0 0\n\
  11       0 sr0 4415 0 311468 1252 0 0 0 0 0 1068 1252 0 0 0 0 0 0\n";
        let (r, w) = parse_diskstats(text, |n| n == "vda" || n == "loop0" || n == "sr0");
        assert_eq!((r, w), (1_308_022 * 512, 7_051_016 * 512));
        assert_eq!(parse_diskstats("garbage", |_| true), (0, 0));
    }

    #[test]
    fn systemd_units_map_main_pid_to_app() {
        let text = "MainPID=4242\nId=app-radio-api-rs.service\n\nMainPID=0\nId=app-landing.service\n\nMainPID=7\nId=other.service\n";
        let m = parse_app_units(text);
        assert_eq!(m.len(), 1);
        assert_eq!(m[&4242], "radio-api-rs");
    }

    #[test]
    fn top_processes_keep_idle_apps() {
        let p = |pid, cpu, app: Option<&str>| Proc {
            pid,
            name: "x".into(),
            app: app.map(Into::into),
            cpu,
            mem_bytes: 1,
            uptime_secs: 1,
        };
        let all: Vec<Proc> = (0..20)
            .map(|i| p(i, i as f64, None))
            .chain([p(99, 0.0, Some("landing"))])
            .collect();
        let top = pick_top(all, 3);
        assert_eq!(top.iter().map(|p| p.pid).collect::<Vec<_>>(), [19, 18, 17, 99]);
    }

    #[test]
    fn versions_and_interfaces() {
        assert_eq!(
            first_version("nginx version: nginx/1.24.0 (Ubuntu)").as_deref(),
            Some("1.24.0")
        );
        assert_eq!(
            first_version("psql (PostgreSQL) 16.15 (Ubuntu 16.15-0ubuntu0.24.04.1)").as_deref(),
            Some("16.15")
        );
        assert_eq!(first_version("certbot 2.9.0").as_deref(), Some("2.9.0"));
        assert_eq!(first_version("no numbers"), None);
        for n in ["lo", "lo0", "docker0", "veth12", "br-ab"] {
            assert!(is_virtual_iface(n), "{n}");
        }
        for n in ["eth0", "en0", "ens3", "local0x"] {
            assert!(!is_virtual_iface(n), "{n}");
        }
    }

    #[tokio::test]
    async fn rollup_averages_finished_buckets_and_prunes() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let base = 1_000_000_200; // multiple of 300
        for i in 0..30 {
            let mut x = s(base + i * 10, if i < 15 { 10.0 } else { 30.0 }, 100.0);
            x.mem_used = 50.0;
            x.mem_cache = 20.0;
            insert(&db, &x, &json!({ "cores": 2, "memCache": 20, "iface": "eth0" }), None)
                .await
                .unwrap();
        }
        // one more sample in the next (unfinished) bucket
        insert(&db, &s(base + 300, 99.0, 0.0), &json!({}), None).await.unwrap();
        // a stale row older than 24 h
        insert(&db, &s(base - 2 * KEEP_10S, 1.0, 0.0), &json!({}), None)
            .await
            .unwrap();

        rollup_and_prune(&db, base + 305).await.unwrap();
        let r = rows_between(&db, true, 0, i64::MAX).await.unwrap();
        assert_eq!(r.len(), 1);
        let a = r[0].sample();
        assert_eq!(
            (a.ts, a.cpu, a.cpu_max, a.net_rx, a.mem_cache),
            (base, 20.0, 30.0, 100.0, 20.0)
        );
        let n: i64 = sqlx::query_scalar("SELECT samples FROM metrics_5m")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(n, 30);
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metrics_10s")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(left, 31); // the stale row is gone

        // running again is a no-op for the finished bucket
        rollup_and_prune(&db, base + 305).await.unwrap();
        assert_eq!(rows_between(&db, true, 0, i64::MAX).await.unwrap().len(), 1);
    }
}
