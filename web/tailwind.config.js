/** @type {import('tailwindcss').Config} */
// The palette is a test report printed light on black: ink-black ground, paper-coloured type. Colour is kept for what a test
// recorded: vermilion for burned, green for did not ignite, ochre for went out. Cobalt is the
// one interface colour (links, the selected control). Nothing is rounded.
module.exports = {
  content: ["./templates/**/*.html", "./src/**/*.rs", "./static/*.js"],
  theme: {
    borderRadius: { none: "0", sm: "0", DEFAULT: "0", md: "0", lg: "0", xl: "0", "2xl": "0", "3xl": "0", full: "9999px" },
    extend: {
      colors: {
        // The names are kept from the first design so templates read the same; the numbers
        // run from the page ground (950) to the lightest tint (700).
        ink: { 950: "#0e0e0c", 900: "#0e0e0c", 850: "#161613", 800: "#1e1e1a", 700: "#272722" },
        line: "#3b3930",
        rule: "#ece8dd",
        fg: "#ece8dd",
        muted: "#a8a496",
        dim: "#858174",
        sky: "#7f93ff",
        blue: "#7f93ff",
        amber: "#e0a03c",
        cyan: "#58b8c6",
        rowhl: "#1b2150",
        burn: "#ff5a26",
        safe: "#4cc27a",
        chamber: "#000000",
      },
      fontFamily: {
        sans: ["Archivo", "system-ui", "sans-serif"],
        mono: ["IBM Plex Mono", "ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
      maxWidth: { page: "1440px" },
    },
  },
};
