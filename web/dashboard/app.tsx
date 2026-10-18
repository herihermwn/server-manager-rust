import { createRoot } from "react-dom/client";
import { AuthProvider, useAuth } from "./lib/auth";
import { parseRoute, useHashRoute } from "./lib/router";
import { Layout } from "./components/Layout";
import { Spinner } from "./components/icons";
import { Login } from "./pages/login";
import { Overview } from "./pages/overview";
import { Usage } from "./pages/usage";
import { Apps } from "./pages/apps";
import { AppNew } from "./pages/apps/new";
import { AppDetail, type AppTab } from "./pages/apps/detail";
import { Databases } from "./pages/databases";
import { Nginx } from "./pages/nginx";
import { NginxSite } from "./pages/nginx/site";
import { Settings } from "./pages/settings";

/**
 * Hash routes. Each page lives in its own file under pages/<area>/ so phases edit disjoint files.
 *   #/                         Overview          (phase 2)
 *   #/usage                    Server usage      (phase 2)
 *   #/apps, #/apps/new         Apps, wizard      (phase 3)
 *   #/apps/<name>[/env|/settings]  App detail    (phase 3; env tab phase 4)
 *   #/databases                Databases         (phase 5)
 *   #/nginx, #/nginx/<id|new>  Web server        (phase 6)
 *   #/settings                 Settings          (phase 7)
 */
function page(route: string) {
  const [first = "", id, sub] = parseRoute(route).segments;
  switch (first) {
    case "":
      return <Overview />;
    case "usage":
      return <Usage />;
    case "apps":
      if (id === "new") return <AppNew />;
      if (id) return <AppDetail key={id} name={id} tab={(sub === "env" || sub === "settings" ? sub : "overview") as AppTab} />;
      return <Apps />;
    case "databases":
      return <Databases />;
    case "nginx":
      return id ? <NginxSite key={id} id={id} /> : <Nginx />;
    case "settings":
      return <Settings />;
    default:
      return <Overview />;
  }
}

function App() {
  const { user } = useAuth();
  const route = useHashRoute();
  if (user === undefined) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-paper text-ink-2" role="status" aria-label="Loading">
        <Spinner size={24} />
      </div>
    );
  }
  if (!user) return <Login />;
  return <Layout currentPath={route}>{page(route)}</Layout>;
}

createRoot(document.getElementById("root")!).render(
  <AuthProvider>
    <App />
  </AuthProvider>,
);
