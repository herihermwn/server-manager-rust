import type { ReactNode } from "react";
import { Card } from "./Card";
import { EmptyState } from "./Feedback";
import { PageHeader } from "./PageHeader";

/** Phase 1 stand-in for a page that a later phase builds. Delete the usage when the page is real. */
export function Placeholder({ title, phase, screens, children }: { title: string; phase: number; screens: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col gap-6">
      <PageHeader title={title} />
      <Card>
        <EmptyState title={`Built in phase ${phase}`} description={<>Design: <span className="font-mono">{screens}</span></>} />
        {children}
      </Card>
    </div>
  );
}
