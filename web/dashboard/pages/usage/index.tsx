import { useState } from "react";
import { Banner, ErrorState } from "../../components/Feedback";
import { PageHeader } from "../../components/PageHeader";
import { SegmentedControl } from "../../components/FilterTabs";
import { LiveDot } from "../../components/Badge";
import { BarList, Section } from "../../components/Card";
import { DataTable, type Column } from "../../components/DataTable";
import { Tag } from "../../components/Badge";
import { UsageChart, type ChartCol } from "../../components/UsageChart";
import { UsageKpiStrip, type UsageKpi } from "../../components/UsageKpiStrip";
import { usePoll, useNowSecs } from "../../components/usePoll";
import { bytes, bytesParts, clock, day, duration, levels, niceCeil, pct, rate, rateParts, type Level } from "./format";
import { DEFAULT_THRESHOLDS, type AlertsResponse, type Current, type DiskUsage, type Proc, type Sample, type Series } from "./types";

type Range = "1h" | "24h" | "7d" | "30d";
const RANGES: { value: Range; label: string }[] = [
  { value: "1h", label: "1 h" },
  { value: "24h", label: "24 h" },
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
];
const RANGE_TEXT: Record<Range, string> = { "1h": "last hour", "24h": "last 24 hours", "7d": "last 7 days", "30d": "last 30 days" };

const BLUE = "#1D5FBF";
const AMBER = "#E9A15C";
const CACHE = "#B8CFF2";
const RED = "#B3261E";

/** Put each point in its slot; slots without a sample stay null (a gap in the chart). */
function toSlots(s: Series): (Sample | null)[] {
  const out: (Sample | null)[] = Array(s.slots).fill(null);
  for (const p of s.points) {
    const i = Math.floor((p.ts - s.from) / s.step);
    if (i >= 0 && i < s.slots) out[i] = p;
  }
  return out;
}

/** Axis labels: "10.0 MB/s" → "10 MB/s". */
const ax = (v: string) => v.replace(".0 ", " ");
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

export function Usage() {
  const [range, setRange] = useState<Range>("1h");
  const cur = usePoll<Current>("/api/metrics/current", 10_000);
  const series = usePoll<Series>(`/api/metrics/series?range=${range}`, range === "1h" ? 10_000 : 30_000);
  const procs = usePoll<Proc[]>("/api/metrics/processes", 10_000);
  const disk = usePoll<DiskUsage>("/api/metrics/disk-usage", 60_000);
  const alerts = usePoll<AlertsResponse>("/api/metrics/alerts", 10_000);
  const nowSecs = useNowSecs();

  const c = cur.data;
  const s = c?.sample ?? null;
  const t = alerts.data?.thresholds ?? DEFAULT_THRESHOLDS;
  const age = s ? (c?.ageSecs ?? 0) + Math.max(0, nowSecs - Math.floor(cur.updatedAt / 1000)) : 0;
  const offline = !!s && (age > 30 || !!cur.error);
  const bannerAlerts = offline ? [] : (alerts.data?.alerts ?? []);
  const sr = series.data;
  const slots = sr ? toSlots(sr) : [];

  const retry = () => {
    cur.reload();
    series.reload();
    procs.reload();
    disk.reload();
    alerts.reload();
  };

  const header = (
    <PageHeader
      title="Server usage"
      subtitle="The manager samples every 10 s and keeps 30 days. Times are WIB."
      actions={
        <>
          <span role="status" className="inline-flex items-center text-ink-2">
            {s ? (
              offline ? (
                <LiveDot tone="danger">Last sample {clock(s.ts, true)}</LiveDot>
              ) : (
                <LiveDot tone="success">Live · updated {Math.round(age)} s ago</LiveDot>
              )
            ) : (
              <LiveDot tone="neutral">{cur.loaded ? "Waiting for the first sample" : "Connecting"}</LiveDot>
            )}
          </span>
          <SegmentedControl label="Time range" options={RANGES} value={range} onChange={setRange} />
        </>
      }
    />
  );

  if (cur.error && !cur.data) {
    return (
      <div className="flex flex-col gap-6">
        {header}
        <ErrorState title="Could not load server usage" description={cur.error.message} onRetry={retry} />
      </div>
    );
  }

  // KPI strip
  const cores = c?.cores ?? 1;
  const kpis: UsageKpi[] = s
    ? (() => {
        const memPct = pct(s.memUsed, s.memTotal);
        const diskPct = pct(s.diskUsed, s.diskTotal);
        const [memV, memU] = bytesParts(s.memUsed);
        const [ioV, ioU] = rateParts(s.diskRead + s.diskWrite);
        const [rxV, rxU] = rateParts(s.netRx);
        const cpuLevel: Level = levels.cpu(s.cpu, t);
        return [
          {
            label: "CPU",
            value: Math.round(s.cpu),
            unit: "%",
            meta: `${plural(cores, "core")}${c?.peak ? ` · peak ${Math.round(c.peak.cpu)}% at ${clock(c.peak.ts)}` : ""}`,
            level: cpuLevel,
          },
          {
            label: "Memory",
            value: memV,
            unit: `${memU} / ${bytes(s.memTotal)}`,
            meta: s.swapTotal > 0 ? `Swap ${bytes(s.swapUsed)} / ${bytes(s.swapTotal)}` : `Cache ${bytes(s.memCache)} · no swap`,
            level: levels.mem(memPct, t),
          },
          { label: "Disk", value: Math.round(diskPct), unit: "%", meta: `${bytes(s.diskTotal - s.diskUsed)} of ${bytes(s.diskTotal)} free`, level: levels.disk(diskPct, t) },
          { label: "Disk I/O", value: ioV, unit: ioU, meta: `write ${rate(s.diskWrite)} · read ${rate(s.diskRead)}` },
          { label: "Network", value: rxV, unit: `${rxU} in`, meta: `out ${rate(s.netTx)}` },
          { label: "Load", value: s.load1.toFixed(2), unit: "1 min", meta: `5 min ${s.load5.toFixed(2)} · ${plural(cores, "core")}`, level: levels.load(s.load1, cores) },
        ];
      })()
    : ["CPU", "Memory", "Disk", "Disk I/O", "Network", "Load"].map((label) => ({ label, value: "–" }));

  // charts
  const memTotal = s?.memTotal ?? sr?.points.at(-1)?.memTotal ?? 0;
  const tip = (p: Sample, text: string) => `${sr && sr.step >= 1800 ? `${day(p.ts)} ` : ""}${clock(p.ts)} · ${text}`;
  const span = sr ? sr.slots * sr.step : 0;
  const axis = sr ? [0, 1, 2, 3, 4].map((i) => (sr.step >= 1800 ? day(sr.from + (span * i) / 4) : clock(sr.from + (span * i) / 4))) : [];
  const gaps = (f: (p: Sample) => ChartCol) => slots.map((p) => (p ? f(p) : null));
  const noData = offline && s ? `No data since ${clock(s.ts)}` : undefined;
  const chartsLoading = !series.loaded;

  const cpuCols = gaps((p) => ({ a: p.cpu, hot: p.cpu >= t.cpuPct, tip: tip(p, `${Math.round(p.cpu)}%`) }));
  const hotMinutes = Math.round((slots.filter((p) => p && p.cpu >= t.cpuPct).length * (sr?.step ?? 0)) / 60);
  const memCols = gaps((p) => ({ a: p.memUsed, b: p.memCache, hot: pct(p.memUsed, p.memTotal) >= t.memWarn, tip: tip(p, `used ${bytes(p.memUsed)} · cache ${bytes(p.memCache)}`) }));
  const netMax = niceCeil(Math.max(0, ...slots.map((p) => (p ? Math.max(p.netRx, p.netTx) : 0))));
  const netCols = gaps((p) => ({ a: p.netRx, b: p.netTx, tip: tip(p, `in ${rate(p.netRx)} · out ${rate(p.netTx)}`) }));
  const ioMax = niceCeil(Math.max(0, ...slots.map((p) => (p ? p.diskRead + p.diskWrite : 0))));
  const ioCols = gaps((p) => ({ a: p.diskRead + p.diskWrite, tip: tip(p, `read ${rate(p.diskRead)} · write ${rate(p.diskWrite)}`) }));

  const chartErr = series.error && !series.data;

  // tables
  const procCols: Column<Proc>[] = [
    {
      key: "name",
      header: "Process",
      render: (p) => (
        <div className="flex flex-col py-1">
          <span className="font-mono text-sm font-medium text-ink">{p.app ?? p.name}</span>
          {p.app && (
            <span className="mt-0.5">
              <Tag>systemd app · {p.name}</Tag>
            </span>
          )}
        </div>
      ),
    },
    {
      key: "cpu",
      header: "CPU",
      align: "right",
      render: (p) => {
        const hot = p.cpu >= t.cpuPct;
        return (
          <span className="inline-flex items-center gap-2">
            <span className="inline-block h-1.5 w-14 overflow-hidden rounded-full bg-muted" aria-hidden="true">
              <span className={`block h-full ${hot ? "bg-danger" : "bg-chart-1"}`} style={{ width: `${Math.min(100, p.cpu / cores)}%` }} />
            </span>
            <span className={`w-14 font-mono text-13 font-medium ${hot ? "text-danger" : "text-ink"}`}>{p.cpu.toFixed(1)}%</span>
          </span>
        );
      },
    },
    { key: "mem", header: "Memory", align: "right", mono: true, render: (p) => bytes(p.memBytes) },
    { key: "pid", header: "PID", align: "right", mono: true, render: (p) => String(p.pid) },
    { key: "up", header: "Uptime", align: "right", mono: true, render: (p) => duration(p.uptimeSecs) },
  ];

  const diskTotal = s ? `${bytes(s.diskUsed)} / ${bytes(s.diskTotal)}` : "";
  const diskLevel = s ? levels.disk(pct(s.diskUsed, s.diskTotal), t) : "ok";
  const diskItems = disk.data?.at
    ? [...disk.data.entries.map((e) => ({ label: e.label, value: e.bytes, display: bytes(e.bytes) })), ...(disk.data.otherBytes != null ? [{ label: "system + other", value: disk.data.otherBytes, display: bytes(disk.data.otherBytes) }] : [])].sort((a, b) => b.value - a.value)
    : [];

  return (
    <div className="flex flex-col gap-6">
      {header}

      {offline && s && (
        <Banner tone="error" title={`No data since ${clock(s.ts)}.`} action={{ label: "Try again", onClick: retry }}>
          The numbers below are the last sample. Check it on the server: <span className="font-mono">systemctl status server-manager</span>
        </Banner>
      )}
      {bannerAlerts.length > 0 && (
        <div className="flex flex-col gap-2">
          {bannerAlerts.map((a) => (
            <Banner key={a.key} tone={a.level === "critical" ? "error" : "warning"} title={a.title}>
              {a.detail}
            </Banner>
          ))}
        </div>
      )}

      <UsageKpiStrip label="Current usage" items={kpis} loading={!cur.loaded} />

      {chartErr ? (
        <ErrorState title="Could not load the charts" description={series.error?.message} onRetry={series.reload} />
      ) : (
        <div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,440px),1fr))] gap-6">
          <UsageChart
            title="CPU"
            kind="bars"
            cols={cpuCols}
            max={100}
            yLabels={["100%", "50%", "0"]}
            axis={axis}
            loading={chartsLoading}
            notice={noData}
            ariaLabel={`CPU usage over the ${RANGE_TEXT[range]}${s ? `, now ${Math.round(s.cpu)}%` : ""}`}
            legend={[
              { color: BLUE, label: "Used", value: s ? `${Math.round(s.cpu)}%` : "–" },
              { color: RED, label: `Above ${t.cpuPct}%`, value: hotMinutes ? duration(hotMinutes * 60) : "0" },
            ]}
          />
          <UsageChart
            title="Memory"
            kind="stacked"
            cols={memCols}
            max={memTotal}
            yLabels={[ax(bytes(memTotal)), ax(bytes(memTotal / 2)), "0"]}
            axis={axis}
            colorB="bg-[#B8CFF2]"
            loading={chartsLoading}
            notice={noData}
            ariaLabel={`Memory used and cache over the ${RANGE_TEXT[range]}${s ? `, used ${bytes(s.memUsed)} of ${bytes(s.memTotal)}` : ""}`}
            legend={[
              { color: BLUE, label: "Used", value: s ? bytes(s.memUsed) : "–" },
              { color: CACHE, label: "Cache", value: s ? bytes(s.memCache) : "–" },
            ]}
          />
          <UsageChart
            title="Network"
            kind="mirror"
            cols={netCols}
            max={netMax}
            yLabels={[ax(rate(netMax)), "0", ax(rate(netMax))]}
            axis={axis}
            loading={chartsLoading}
            notice={noData}
            ariaLabel={`Network in and out over the ${RANGE_TEXT[range]}${s ? `, in ${rate(s.netRx)}, out ${rate(s.netTx)}` : ""}`}
            legend={[
              { color: BLUE, label: "In", value: s ? rate(s.netRx) : "–" },
              { color: AMBER, label: "Out", value: s ? rate(s.netTx) : "–" },
            ]}
          />
          <UsageChart
            title="Disk I/O"
            kind="bars"
            colorA="bg-chart-2"
            cols={ioCols}
            max={ioMax}
            yLabels={[ax(rate(ioMax)), ax(rate(ioMax / 2)), "0"]}
            axis={axis}
            loading={chartsLoading}
            notice={noData}
            ariaLabel={`Disk read and write over the ${RANGE_TEXT[range]}${s ? `, now ${rate(s.diskRead + s.diskWrite)}` : ""}`}
            legend={[{ color: AMBER, label: "Read + write", value: s ? rate(s.diskRead + s.diskWrite) : "–" }]}
          />
        </div>
      )}

      <div className="flex flex-wrap items-start gap-6">
        <div className="min-w-0 flex-[2_1_560px]">
          <Section
            flush
            title="Processes"
            action={<span className="text-13 text-ink-2">Sorted by CPU · 100% = one core</span>}
          >
            {procs.error && !procs.data ? (
              <div className="p-5">
                <ErrorState title="Could not load processes" description={procs.error.message} onRetry={procs.reload} />
              </div>
            ) : (
              <DataTable<Proc> framed={false} label="Processes" columns={procCols} data={procs.data ?? []} loading={!procs.loaded} skeletonRows={6} rowKey={(p) => p.pid} emptyMessage="No process data" minWidth={640} />
            )}
          </Section>
        </div>

        <div className="flex min-w-0 flex-[1_1_320px] flex-col gap-6">
          <Section title="What uses disk" subtitle={disk.data?.at ? `Folder sizes, measured ${clock(disk.data.at)}` : undefined} action={s && <span className={`font-mono text-13 font-medium ${diskLevel === "bad" ? "text-danger" : diskLevel === "warn" ? "text-warning" : "text-ink-2"}`}>{diskTotal}</span>}>
            {disk.error && !disk.data ? (
              <ErrorState title="Could not load disk usage" description={disk.error.message} onRetry={disk.reload} />
            ) : !disk.loaded ? (
              <p className="text-13 text-ink-2">Loading folder sizes…</p>
            ) : diskItems.length === 0 ? (
              <p className="text-13 text-ink-2">Measuring folder sizes. The first result takes under a minute; after that they refresh every 10 minutes.</p>
            ) : (
              <BarList items={diskItems} labelWidth={150} />
            )}
          </Section>

          <Section title="Network today" subtitle="Since 00:00 WIB">
            <dl className="flex flex-col">
              {[
                [`Received${c?.iface ? ` (${c.iface})` : ""}`, c?.today ? bytes(c.today.rx) : "–"],
                [`Sent${c?.iface ? ` (${c.iface})` : ""}`, c?.today ? bytes(c.today.tx) : "–"],
                ["Peak in", c?.today ? rate(c.today.peakRx) : "–"],
                ["Peak out", c?.today ? rate(c.today.peakTx) : "–"],
              ].map(([k, v], i) => (
                <div key={k} className={`flex min-h-9 items-center justify-between gap-3 text-sm ${i ? "border-t border-line-soft" : ""}`}>
                  <dt className="text-ink-2">{k}</dt>
                  <dd className="font-mono text-13 font-medium text-ink">{v}</dd>
                </div>
              ))}
            </dl>
          </Section>
        </div>
      </div>
    </div>
  );
}
