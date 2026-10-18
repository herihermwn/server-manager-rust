/**
 * API calls. Auth is the HttpOnly `sm_session` cookie (Path=/api), which the browser sends by
 * itself, so there is no token in JS. A 401 from any call fires "auth:expired" and the app
 * returns to the sign-in page.
 */

export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
    public body: Record<string, unknown> = {},
  ) {
    super(message);
  }
}

/** fetch() for /api URLs: same-origin cookies, 401 → sign-in page. */
export async function authFetch(url: string, options: RequestInit = {}): Promise<Response> {
  const res = await fetch(url, { credentials: "same-origin", ...options });
  if (res.status === 401 && url !== "/api/auth/login") window.dispatchEvent(new Event("auth:expired"));
  return res;
}

/**
 * JSON in, JSON out. Throws ApiError with the server's `error` text on non-2xx.
 *   const apps = await api<App[]>("/api/apps");
 *   await api("/api/apps", { method: "POST", body: { name } });
 */
export async function api<T = unknown>(
  url: string,
  opts: { method?: string; body?: unknown; signal?: AbortSignal } = {},
): Promise<T> {
  const init: RequestInit = { method: opts.method ?? (opts.body === undefined ? "GET" : "POST"), signal: opts.signal };
  if (opts.body !== undefined) {
    init.headers = { "Content-Type": "application/json" };
    init.body = JSON.stringify(opts.body);
  }
  const res = await authFetch(url, init);
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = { error: text };
  }
  if (!res.ok) {
    const body = (data ?? {}) as Record<string, unknown>;
    throw new ApiError(typeof body.error === "string" ? body.error : `Request failed (${res.status})`, res.status, body);
  }
  return data as T;
}
