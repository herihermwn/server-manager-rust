import { Card } from "../../../components/Card";
import { EmptyState } from "../../../components/Feedback";

/** Phase 3 builds this tab. Design: AppDetail, AppDeploying, AppDeployFailed, AppCrashed */
export function AppOverviewTab({ name }: { name: string }) {
  return (
    <Card>
      <EmptyState title="Built in phase 3" description={<>Design: <span className="font-mono">AppDetail, AppDeploying, AppDeployFailed, AppCrashed</span> for {name}</>} />
    </Card>
  );
}
