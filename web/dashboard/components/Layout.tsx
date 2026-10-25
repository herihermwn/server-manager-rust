import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { Sidebar, crumbFor } from "./Sidebar";
import { MobileBar, Topbar } from "./Topbar";
import { ToastProvider } from "./Toast";

const CrumbContext = createContext<(label: string | undefined) => void>(() => {});

/**
 * Detail pages: name the last breadcrumb part once data is loaded.
 * usePageCrumb(app?.name)  →  "Apps › radio-api-rs" (falls back to the id in the URL).
 */
export function usePageCrumb(label: string | undefined) {
  const set = useContext(CrumbContext);
  useEffect(() => {
    set(label);
    return () => set(undefined);
  }, [label]);
}

interface LayoutProps {
  children: ReactNode;
  currentPath: string;
}

export function Layout({ children, currentPath }: LayoutProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [crumbLabel, setCrumbLabel] = useState<string | undefined>();
  const main = useRef<HTMLElement>(null);
  const crumb = crumbFor(currentPath, crumbLabel);

  useEffect(() => {
    setMenuOpen(false);
    window.scrollTo(0, 0);
  }, [currentPath.split("?")[0]]);

  return (
    <ToastProvider>
      <CrumbContext.Provider value={setCrumbLabel}>
        <button
          type="button"
          onClick={() => main.current?.focus()}
          className="sr-only z-[70] rounded-lg bg-ink px-4 py-3 text-sm font-semibold text-ink-on focus:not-sr-only focus:fixed focus:left-4 focus:top-4"
        >
          Skip to content
        </button>
        <div className="flex min-h-screen bg-paper">
          <Sidebar currentPath={currentPath} isOpen={menuOpen} onClose={() => setMenuOpen(false)} />
          <div className="flex min-w-0 flex-1 flex-col">
            <MobileBar title={crumb.page} menuOpen={menuOpen} onMenu={() => setMenuOpen(true)} />
            <Topbar crumb={crumb} />
            <main ref={main} tabIndex={-1} className="w-full p-4 focus-visible:outline-none lg:p-8">
              {children}
            </main>
          </div>
        </div>
      </CrumbContext.Provider>
    </ToastProvider>
  );
}
