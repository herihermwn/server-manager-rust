import { usePageCrumb } from "../../../components/Layout";
import { Tag } from "../../../components/Badge";
import { AppOverviewTab } from "./overview";
import { AppEnvTab } from "./env";
import { AppSettingsTab } from "./settings";

export type AppTab = "overview" | "env" | "settings";

const TABS: { key: AppTab; label: string; suffix: string }[] = [
  { key: "overview", label: "Overview & deploys", suffix: "" },
  { key: "env", label: "Environment", suffix: "/env" },
  { key: "settings", label: "Settings", suffix: "/settings" },
];

/**
 * AppHeader.dc.html: name, status, runtime · systemd, repo/path/port line, actions, tabs.
 * Phase 3 owns this shell (header data + actions) and the overview/settings tabs; phase 4 owns env.tsx.
 * Routes: #/apps/<name>, #/apps/<name>/env, #/apps/<name>/settings.
 */
export function AppDetail({ name, tab }: { name: string; tab: AppTab }) {
  usePageCrumb(name);
  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-2.5">
        <div className="flex flex-wrap items-center gap-3">
          <h1 className="text-28 font-semibold">{name}</h1>
          <Tag>systemd</Tag>
        </div>
      </div>
      <nav aria-label="App sections" className="flex gap-1 overflow-x-auto border-b border-line">
        {TABS.map((t) => {
          const on = t.key === tab;
          return (
            <a
              key={t.key}
              href={`#/apps/${encodeURIComponent(name)}${t.suffix}`}
              aria-current={on ? "page" : undefined}
              className={`-mb-px inline-flex min-h-11 items-center gap-2 whitespace-nowrap border-b-2 px-3.5 text-sm no-underline ${
                on ? "border-ink font-semibold text-ink" : "border-transparent font-medium text-ink-2 hover:text-ink"
              }`}
            >
              {t.label}
            </a>
          );
        })}
      </nav>
      {tab === "overview" && <AppOverviewTab name={name} />}
      {tab === "env" && <AppEnvTab name={name} />}
      {tab === "settings" && <AppSettingsTab name={name} />}
    </div>
  );
}
