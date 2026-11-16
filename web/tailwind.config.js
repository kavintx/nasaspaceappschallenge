/** @type {import('tailwindcss').Config} */
// The palette is paper and ink, like a printed test report. Colour is kept for what a test
// recorded: vermilion for burned, green for did not ignite, ochre for went out. Cobalt is the
// one interface colour (links, the selected control). Nothing is rounded.
module.exports = {
  content: ["./templates/**/*.html", "./src/**/*.rs", "./static/*.js"],
  theme: {
    borderRadius: { none: "0", sm: "0", DEFAULT: "0", md: "0", lg: "0", xl: "0", "2xl": "0", "3xl": "0", full: "9999px" },
    extend: {
      colors: {
        // The names are kept from the first design so templates read the same; the numbers
        // now run from paper (950) to the darkest tint (700).
        ink: { 950: "#efece3", 900: "#efece3", 850: "#e7e3d7", 800: "#dfdacb", 700: "#d5cfbe" },
        line: "#b3ad9c",
        rule: "#14130f",
        fg: "#14130f",
        muted: "#4f4c43",
        dim: "#6f6b61",
        sky: "#1a35d6",
        blue: "#1a35d6",
        amber: "#8f5300",
        cyan: "#0a6a78",
        rowhl: "#dbe0fb",
        burn: "#d9390a",
        safe: "#0c7a3d",
        chamber: "#0b0b0d",
      },
      fontFamily: {
        sans: ["Archivo", "system-ui", "sans-serif"],
        mono: ["IBM Plex Mono", "ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
      maxWidth: { page: "1440px" },
    },
  },
};
