import { useEffect, useState } from "react";

/** Hash route without the leading "#", e.g. "/apps/radio-api-rs/env?x=1". */
export function useHashRoute() {
  const read = () => window.location.hash.slice(1) || "/";
  const [route, setRoute] = useState(read);
  useEffect(() => {
    const on = () => setRoute(read());
    window.addEventListener("hashchange", on);
    return () => window.removeEventListener("hashchange", on);
  }, []);
  return route;
}

/** "/apps/x/env?tab=1" → { segments: ["apps", "x", "env"], query: URLSearchParams } */
export function parseRoute(route: string) {
  const [path = "", qs = ""] = route.split("?");
  return { segments: path.split("/").filter(Boolean).map(decodeURIComponent), query: new URLSearchParams(qs) };
}

export function navigate(path: string) {
  window.location.hash = path;
}
