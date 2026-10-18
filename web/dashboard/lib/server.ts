// The one server this manager runs on (README "Server"). Shown on the sign-in page and in the sidebar.
export const SERVER = {
  host: "43.156.104.214",
  user: "ubuntu",
  specs: "Ubuntu 24.04 LTS · 2 vCPU · 3.6 GB",
};

/** Times in the UI are WIB (server clock is Asia/Shanghai, the API sends unix seconds). */
export function formatWib(unixSecs: number, opts: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" }) {
  return new Date(unixSecs * 1000).toLocaleString("en-GB", { timeZone: "Asia/Jakarta", ...opts });
}
