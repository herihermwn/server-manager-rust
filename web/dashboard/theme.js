// Saved choice, else the OS setting. Classic script in <head> (CSP blocks inline) so there is no light flash.
try {
  document.documentElement.dataset.theme =
    localStorage.getItem("theme") || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
} catch {}
