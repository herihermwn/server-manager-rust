import defaultTheme from "tailwindcss/defaultTheme";

/** Tokens from dashboard-design/Main.dc.html (same system as the Radio API console). No purple. */
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
        overlay: "0 12px 32px rgba(27, 26, 23, 0.16), 0 0 0 1px rgba(27, 26, 23, 0.08)",
        modal: "0 24px 64px rgba(27, 26, 23, 0.28)",
        toast: "0 12px 32px rgba(27, 26, 23, 0.24)",
        tab: "0 1px 2px rgba(27, 26, 23, 0.12)",
      },
      colors: {
        paper: "#F4F3EF", // app ground
        surface: "#FFFFFF", // cards, inputs
        sunken: "#F9F8F5", // table header, modal footer
        muted: "#EEEDE8", // neutral fills: tags, skeleton, tab track
        line: {
          DEFAULT: "#E3E1DA", // borders, rules
          strong: "#CFCCC3", // input / secondary button border
          soft: "#EEEDE8", // table row dividers
        },
        ink: {
          DEFAULT: "#1B1A17", // text, primary button
          2: "#47453F", // secondary text
          3: "#6A675F", // captions, column labels (5.3:1)
          hover: "#3A3833", // primary button hover
        },
        disabled: { DEFAULT: "#8F8B83", bg: "#EEEDE8" },
        console: {
          DEFAULT: "#1B1A17", // sidebar, terminal ground
          raised: "#34322D", // active nav item, rules
          text: "#E9E7E1",
          dim: "#C9C6BE",
          muted: "#9C988F",
        },
        // Terminal line colors (DataDisplay.dc.html log)
        term: { cmd: "#9DC0F5", ok: "#8FD3A8", warn: "#F5C27A", err: "#F29B94", live: "#6FA3EE" },
        brand: { DEFAULT: "#C2372F", soft: "#F0716A" }, // signal only: logo, live, active page, attention count
        info: {
          DEFAULT: "#1D5FBF", // links, focus, chart 1, deploying
          bg: "#E6EEFA",
          hover: "#D5E3F7",
          strong: "#164A96",
          ink: "#123E7D",
          ring: "#CFE0F8",
        },
        chart: { 1: "#1D5FBF", 2: "#E9A15C" },
        success: { DEFAULT: "#146C43", bg: "#E5F2EA" },
        warning: { DEFAULT: "#8A5300", bg: "#FBEFD9", ink: "#5C3700", line: "#D9B77A" },
        danger: { DEFAULT: "#B3261E", bg: "#FBE8E6", ink: "#7A1A14", hover: "#931E18" },
        neutral: { DEFAULT: "#47453F", bg: "#EEEDE8" },
      },
    },
  },
  plugins: [],
};
