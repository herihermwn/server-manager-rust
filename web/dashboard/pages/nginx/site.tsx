import { Placeholder } from "../../components/Placeholder";

/** Phase 6 builds this page. `id` is the site id, or "new". Design: NginxSite, NginxTestFailed, NginxSsl */
export function NginxSite({ id }: { id: string }) {
  return <Placeholder title={id === "new" ? "New site" : `Site #${id}`} phase={6} screens="NginxSite, NginxTestFailed, NginxSsl" />;
}
