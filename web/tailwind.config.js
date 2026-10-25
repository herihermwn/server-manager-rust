import defaultTheme from "tailwindcss/defaultTheme";

/** Tokens from dashboard-design/Main.dc.html (same system as the Radio API console). No purple. */
const v = (name) => `rgb(var(--${name}) / <alpha-value>)`;

/** @type {import('tailwindcss').Config} */
export default {
  content: ["./dashboard/**/*.{html,tsx,ts}"],
  theme: {
    extend: {
      fontFamily: {
        sans: ['"IBM Plex Sans"', ...defaultTheme.fontFamily.sans],
        mono: ['"IBM Plex Mono"', ...defaultTheme.fontFamily.mono],
      },
      // text-sm (14/20) and text-xs (12/16) are Tailwind defaults.
      fontSize: {
        13: ["13px", "18px"],
        15: ["15px", "22px"],
        17: ["17px", "24px"],
        22: ["22px", "28px"],
        24: ["24px", "32px"],
        28: ["28px", "36px"],
        32: ["32px", "40px"],
      },
      letterSpacing: { caps: "0.06em" },
      // Radii match Tailwind defaults: rounded-md 6 (badge), rounded-lg 8 (control),
      // rounded-xl 12 (card), rounded-2xl 16 (modal).
      boxShadow: {
        overlay: "0 12px 32px rgb(var(--shadow) / 0.16), 0 0 0 1px rgb(var(--shadow-ring) / 0.08)",
        modal: "0 24px 64px rgb(var(--shadow) / 0.28)",
        toast: "0 12px 32px rgb(var(--shadow) / 0.24)",
        tab: "0 1px 2px rgb(var(--shadow) / 0.12)",
      },
      // Values live in index.css (:root light, [data-theme=dark] dark) as RGB channels so /opacity works.
      colors: {
        paper: v("paper"), // app ground
        surface: v("surface"), // cards, inputs
        sunken: v("sunken"), // table header, modal footer
        muted: v("muted"), // neutral fills: tags, skeleton, tab track
        line: {
          DEFAULT: v("line"), // borders, rules
          strong: v("line-strong"), // input / secondary button border
          soft: v("line-soft"), // table row dividers
        },
        ink: {
          DEFAULT: v("ink"), // text, primary button
          2: v("ink-2"), // secondary text
          3: v("ink-3"), // captions, column labels (5.3:1)
          hover: v("ink-hover"), // primary button hover
          on: v("ink-on"), // text on ink / success / danger fills
        },
        disabled: { DEFAULT: v("disabled"), bg: v("disabled-bg") },
        console: {
          DEFAULT: v("console"), // sidebar, terminal ground
          raised: v("console-raised"), // active nav item, rules
          text: "#E9E7E1",
          dim: "#C9C6BE",
          muted: "#9C988F",
        },
        // Terminal line colors (DataDisplay.dc.html log), always on a dark ground
        term: { cmd: "#9DC0F5", ok: "#8FD3A8", warn: "#F5C27A", err: "#F29B94", live: "#6FA3EE" },
        brand: { DEFAULT: v("brand"), soft: "#F0716A" }, // signal only: logo, live, active page, attention count
        info: {
          DEFAULT: v("info"), // links, focus, chart 1, deploying
          bg: v("info-bg"),
          hover: v("info-hover"),
          strong: v("info-strong"),
          ink: v("info-ink"),
          ring: v("info-ring"),
        },
        chart: { 1: v("info"), 2: v("chart-2"), cache: v("chart-cache") },
        success: { DEFAULT: v("success"), bg: v("success-bg") },
        warning: { DEFAULT: v("warning"), bg: v("warning-bg"), ink: v("warning-ink"), line: v("warning-line") },
        danger: { DEFAULT: v("danger"), bg: v("danger-bg"), ink: v("danger-ink"), hover: v("danger-hover") },
        neutral: { DEFAULT: v("ink-2"), bg: v("muted") },
      },
    },
  },
  plugins: [],
};
