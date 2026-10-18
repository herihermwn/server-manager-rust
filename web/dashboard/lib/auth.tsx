import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { api, ApiError } from "./api";

export interface User {
  username: string;
}

/** Result of a sign-in attempt. `triesLeft` / `retryAfter` (s) come from the per-IP lockout. */
export type LoginResult = { ok: true } | { ok: false; error: string; triesLeft?: number; retryAfter?: number };

interface AuthContextType {
  /** undefined while the session is checked, null when signed out. */
  user: User | null | undefined;
  /** Public base URL of this dashboard (SM_PUBLIC_URL, else the request host), no trailing slash. */
  publicUrl: string;
  login: (username: string, password: string) => Promise<LoginResult>;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthContextType | null>(null);

export function useAuth() {
  const c = useContext(AuthContext);
  if (!c) throw new Error("useAuth must be used within AuthProvider");
  return c;
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<User | null | undefined>(undefined);
  const [publicUrl, setPublicUrl] = useState(() => (window.location.origin + window.location.pathname).replace(/\/[^/]*$/, ""));

  useEffect(() => {
    api<{ user: User; publicUrl?: string }>("/api/auth/me")
      .then((r) => {
        setUser(r.user);
        if (r.publicUrl) setPublicUrl(r.publicUrl);
      })
      .catch(() => setUser(null));
    const expired = () => setUser(null);
    window.addEventListener("auth:expired", expired);
    return () => window.removeEventListener("auth:expired", expired);
  }, []);

  const login = useCallback(async (username: string, password: string): Promise<LoginResult> => {
    try {
      const r = await api<{ user: User; publicUrl?: string }>("/api/auth/login", { body: { username, password } });
      setUser(r.user);
      if (r.publicUrl) setPublicUrl(r.publicUrl);
      return { ok: true };
    } catch (e) {
      if (e instanceof ApiError) {
        const b = e.body as { triesLeft?: number; retryAfter?: number };
        return { ok: false, error: e.message, triesLeft: b.triesLeft, retryAfter: b.retryAfter };
      }
      return { ok: false, error: "Could not reach the server. Is the SSH tunnel open?" };
    }
  }, []);

  const logout = useCallback(async () => {
    await api("/api/auth/logout", { method: "POST" }).catch(() => {});
    setUser(null);
    window.location.hash = "/";
  }, []);

  return <AuthContext.Provider value={{ user, publicUrl, login, logout }}>{children}</AuthContext.Provider>;
}
