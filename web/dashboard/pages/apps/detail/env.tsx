import { Card } from "../../../components/Card";
import { EmptyState } from "../../../components/Feedback";

/** Phase 4 builds this tab. Design: AppEnv, AppEnvEditing, AppEnvImport */
export function AppEnvTab({ name }: { name: string }) {
  return (
    <Card>
      <EmptyState title="Built in phase 4" description={<>Design: <span className="font-mono">AppEnv, AppEnvEditing, AppEnvImport</span> for {name}</>} />
    </Card>
  );
}
