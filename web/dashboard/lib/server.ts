// The server this manager runs on, as the browser reached it. Shown on the sign-in page and in the sidebar.
export const SERVER = { host: window.location.hostname };

/** Times in the UI are WIB (server clock is Asia/Shanghai, the API sends unix seconds). */
export function formatWib(unixSecs: number, opts: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" }) {
  return new Date(unixSecs * 1000).toLocaleString("en-GB", { timeZone: "Asia/Jakarta", ...opts });
}
