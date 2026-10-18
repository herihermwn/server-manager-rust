import { useRef, type ComponentType } from "react";
import { useAuth } from "../lib/auth";
import { SERVER } from "../lib/server";
import { IconButton } from "./Button";
import { CountBadge } from "./Badge";
import { useDialog } from "./Modal";
import {
  AppsIcon,
  BrandTile,
  CloseIcon,
  DatabaseIcon,
  LogoutIcon,
  OverviewIcon,
  SettingsIcon,
  UsageIcon,
  WebServerIcon,
  type IconProps,
} from "./icons";

interface NavItem {
  path: string;
  label: string;
  icon: ComponentType<IconProps>;
}

interface NavGroup {
  /** Breadcrumb section; also the group heading when `heading` is true. */
  section: string;
  heading: boolean;
  items: NavItem[];
}

/** Sidebar.dc.html. Add new top-level pages here and in app.tsx. */
export const NAV: NavGroup[] = [
  {
    section: "Server",
    heading: false,
    items: [
      { path: "/", label: "Overview", icon: OverviewIcon },
      { path: "/usage", label: "Server usage", icon: UsageIcon },
    ],
  },
  { section: "Deploy", heading: true, items: [{ path: "/apps", label: "Apps", icon: AppsIcon }] },
  {
    section: "Server",
    heading: true,
    items: [
      { path: "/databases", label: "Databases", icon: DatabaseIcon },
      { path: "/nginx", label: "Web server", icon: WebServerIcon },
    ],
  },
];

const SETTINGS: NavItem = { path: "/settings", label: "Settings", icon: SettingsIcon };

/** Last crumb for sub-pages whose id is not a record name. */
const SUBPAGE: Record<string, string> = { "apps/new": "New app", "nginx/new": "New site" };

export interface Crumb {
  section: string;
  /** Set on detail pages: the section links back to the list. */
  sectionHref?: string;
  page: string;
}

/** Breadcrumb for a hash route ("/apps/radio-api-rs/env"). `override` replaces the last part. */
export function crumbFor(route: string, override?: string): Crumb {
  const [first = "", id] = (route.split("?")[0] ?? "").split("/").filter(Boolean);
  if (first === "settings") return { section: "Account", page: "Settings" };
  for (const g of NAV) {
    const item = g.items.find((i) => i.path === `/${first}`);
    if (!item) continue;
    if (id) return { section: item.label, sectionHref: `#${item.path}`, page: override ?? SUBPAGE[`${first}/${id}`] ?? decodeURIComponent(id) };
    return { section: g.section, page: override ?? item.label };
  }
  return { section: "Server", page: override ?? "Overview" };
}

function isActive(path: string, route: string) {
  const r = route.split("?")[0] || "/";
  return path === "/" ? r === "/" : r === path || r.startsWith(`${path}/`);
}

function NavLink({ item, route, badge, onNavigate }: { item: NavItem; route: string; badge?: number; onNavigate?: () => void }) {
  const on = isActive(item.path, route);
  const Icon = item.icon;
  return (
    <a
      href={`#${item.path}`}
      onClick={onNavigate}
      aria-current={on ? "page" : undefined}
      className={`flex min-h-10 items-center gap-3 rounded-lg px-3 text-sm no-underline transition-colors ${
        on ? "bg-console-raised font-semibold text-white" : "font-medium text-console-dim hover:bg-console-raised/60 hover:text-white"
      }`}
    >
      <Icon strokeWidth={1.8} />
      <span className="flex-1">{item.label}</span>
      {!!badge && <CountBadge count={badge} tone="brand" label={`${badge} need attention`} />}
      {on && <span className="h-1.5 w-1.5 rounded-full bg-brand-soft" aria-hidden="true" />}
    </a>
  );
}

interface SidebarProps {
  currentPath: string;
  /** Mobile drawer state (below lg). Desktop ignores it. */
  isOpen?: boolean;
  onClose?: () => void;
}

/** Dark console navigation. Static column from lg, slide-in drawer below. */
// ponytail: the Apps attention badge (crashed / failed deploys) is not wired; Phase 3 passes `badge` to its NavLink.
export function Sidebar({ currentPath, isOpen = false, onClose = () => {} }: SidebarProps) {
  const { user, logout } = useAuth();
  const panel = useRef<HTMLElement>(null);
  useDialog(isOpen, onClose, panel);
  const name = user?.username ?? "owner";

  return (
    <>
      {isOpen && <div className="fixed inset-0 z-40 bg-ink/[0.48] lg:hidden" onClick={onClose} aria-hidden="true" />}
      <nav
        ref={panel}
        aria-label="Main"
        className={`fixed inset-y-0 left-0 z-50 flex w-[248px] shrink-0 flex-col gap-6 overflow-y-auto bg-console px-3 pb-4 pt-5 text-console-text transition-transform lg:sticky lg:top-0 lg:z-auto lg:h-screen lg:translate-x-0 ${
          isOpen ? "translate-x-0" : "invisible -translate-x-full lg:visible"
        }`}
      >
        <div className="flex items-center justify-between gap-2">
          <a href="#/" onClick={onClose} className="flex min-h-11 items-center gap-3 px-2 text-white no-underline">
            <BrandTile size={34} />
            <span className="flex min-w-0 flex-col">
              <span className="text-15 font-semibold leading-5">Server Manager</span>
              <span className="font-mono text-xs text-console-muted">{SERVER.host}</span>
            </span>
          </a>
          <IconButton label="Close menu" variant="console" onClick={onClose} className="lg:hidden">
            <CloseIcon />
          </IconButton>
        </div>

        <div className="flex flex-1 flex-col gap-5">
          {NAV.map((g, i) => (
            <div key={i} className="flex flex-col gap-0.5">
              {g.heading && (
                <div className="px-3 pb-1.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-console-muted">{g.section}</div>
              )}
              {g.items.map((it) => (
                <NavLink key={it.path} item={it} route={currentPath} onNavigate={onClose} />
              ))}
            </div>
          ))}
        </div>

        <div className="flex flex-col gap-0.5 border-t border-console-raised pt-3">
          <NavLink item={SETTINGS} route={currentPath} onNavigate={onClose} />
          <div className="flex items-center gap-2.5 pl-3 pr-1 pt-2">
            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-console-raised text-xs font-semibold text-console-text" aria-hidden="true">
              {name.slice(0, 2).toUpperCase()}
            </span>
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate text-sm font-medium">{name}</span>
              <span className="text-xs text-console-muted">Owner</span>
            </span>
            <IconButton label="Log out" variant="console" onClick={logout}>
              <LogoutIcon strokeWidth={1.8} />
            </IconButton>
          </div>
        </div>
      </nav>
    </>
  );
}
