import type { ReactNode } from "react";

/**
 * Inline stroke icons (24px grid, currentColor). Decorative by default (aria-hidden);
 * the control that holds the icon carries the accessible name.
 */
export interface IconProps {
  size?: number;
  strokeWidth?: number;
  className?: string;
}

function Svg({ size = 18, strokeWidth = 2, className, children }: IconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      className={`shrink-0 ${className ?? ""}`}
    >
      {children}
    </svg>
  );
}

const icon = (d: string, extra?: ReactNode) =>
  function Icon(p: IconProps) {
    return (
      <Svg {...p}>
        <path d={d} />
        {extra}
      </Svg>
    );
  };

// Actions
export const PlusIcon = icon("M12 5v14M5 12h14");
export const RefreshIcon = icon("M20 11a8 8 0 0 0-14.9-4M4 4v4h4M4 13a8 8 0 0 0 14.9 4M20 20v-4h-4");
export const FilterIcon = icon("M3 5h18M6 12h12M10 19h4");
export const SearchIcon = icon("M20 20l-4-4", <circle cx="11" cy="11" r="7" />);
export const CloseIcon = icon("M6 6l12 12M18 6L6 18");
export const CheckIcon = icon("M5 12.5l4.5 4.5L19 7");
export const ExternalIcon = icon("M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5");
export const MenuIcon = icon("M4 7h16M4 12h16M4 17h16");
export const LogoutIcon = icon("M15 4h3a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-3M10 16l-4-4 4-4M6 12h10");
export const SunIcon = icon("M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4", <circle cx="12" cy="12" r="4" />);
export const MoonIcon = icon("M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5z");
export const EditIcon = icon("M4 20h4L19 9l-4-4L4 16v4zM13.5 6.5l4 4");
export const TrashIcon = icon("M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3");
export const CopyIcon = icon("M9 9h11v11H9zM5 15H4V4h11v1");

// Chevrons
export const ChevronDownIcon = icon("M6 9l6 6 6-6");
export const ChevronRightIcon = icon("M9 6l6 6-6 6");
export const ChevronLeftIcon = icon("M15 6l-6 6 6 6");

// Visibility
export const EyeIcon = icon("M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z", <circle cx="12" cy="12" r="3" />);
export const EyeOffIcon = icon("M3 3l18 18M10.6 5.1A10 10 0 0 1 12 5c6.5 0 10 7 10 7a17 17 0 0 1-3.2 4M6.6 6.6A17 17 0 0 0 2 12s3.5 7 10 7a9.6 9.6 0 0 0 5.4-1.6M9.9 9.9a3 3 0 0 0 4.2 4.2");

// Feedback
export const AlertCircleIcon = icon("M12 7v6M12 16.5v.5", <circle cx="12" cy="12" r="9" />);
export const AlertTriangleIcon = icon("M12 3l9.5 17h-19L12 3zM12 10v4M12 17v.5");
export const InfoIcon = icon("M12 11v6M12 7.5v.5", <circle cx="12" cy="12" r="9" />);
export const EmptySearchIcon = icon("M20 20l-4-4M8 11h6", <circle cx="11" cy="11" r="7" />);

// Navigation (Sidebar.dc.html)
export const OverviewIcon = icon("M4 4h7v7H4zM13 4h7v4h-7zM13 10h7v10h-7zM4 13h7v7H4z");
export const UsageIcon = icon("M3 12h4l3-8 4 16 3-8h4");
export const AppsIcon = icon("M12 3l8 4.5v9L12 21l-8-4.5v-9zM12 12l8-4.5M12 12v9M12 12L4 7.5");
export const DatabaseIcon = icon("M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zM4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3");
export const WebServerIcon = icon("M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3c2.5 2.5 3.5 5.5 3.5 9s-1 6.5-3.5 9c-2.5-2.5-3.5-5.5-3.5-9s1-6.5 3.5-9z");
export const SettingsIcon = icon("M4 7h10M18 7h2M4 17h4M12 17h8M16 5v4M10 15v4");

// App actions (AppHeader.dc.html)
export const DeployIcon = icon("M12 19V5M5 12l7-7 7 7");
export const RestartIcon = RefreshIcon;
export const BranchIcon = icon("M6 7v10M18 9a6 6 0 0 1-6 6H6", <><circle cx="6" cy="5" r="2" /><circle cx="6" cy="19" r="2" /><circle cx="18" cy="7" r="2" /></>);

/** Three dots, filled. For "More actions" icon buttons. */
export function DotsIcon({ size = 18, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" focusable="false" className={`shrink-0 ${className ?? ""}`}>
      <circle cx="5" cy="12" r="1.8" />
      <circle cx="12" cy="12" r="1.8" />
      <circle cx="19" cy="12" r="1.8" />
    </svg>
  );
}

/** Two stacked server units, used inside the red brand tile. */
export function ServerMarkIcon({ size = 20, strokeWidth = 2, className }: IconProps) {
  return (
    <Svg size={size} strokeWidth={strokeWidth} className={className}>
      <rect x="4" y="4" width="16" height="7" rx="1.5" />
      <rect x="4" y="13" width="16" height="7" rx="1.5" />
      <path d="M8 7.5h.01M8 16.5h.01" />
    </Svg>
  );
}

/** Spinning arc for busy states. */
export function Spinner({ size = 16, className }: IconProps) {
  return (
    <Svg size={size} strokeWidth={2.25} className={`animate-spin ${className ?? ""}`}>
      <path d="M21 12a9 9 0 1 1-6.2-8.6" />
    </Svg>
  );
}

/** Red rounded tile with the server mark (sidebar, login, mobile bar). */
export function BrandTile({ size = 34 }: { size?: number }) {
  return (
    <span
      className="flex shrink-0 items-center justify-center bg-brand text-white"
      style={{ width: size, height: size, borderRadius: Math.round(size * 0.26) }}
    >
      <ServerMarkIcon size={Math.round(size * 0.58)} />
    </span>
  );
}
