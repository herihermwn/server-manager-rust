import { Card } from "../../../components/Card";
import { EmptyState } from "../../../components/Feedback";

/** Phase 3 builds this tab. Design: AppSettings, AppDelete */
export function AppSettingsTab({ name }: { name: string }) {
  return (
    <Card>
      <EmptyState title="Built in phase 3" description={<>Design: <span className="font-mono">AppSettings, AppDelete</span> for {name}</>} />
    </Card>
  );
}
