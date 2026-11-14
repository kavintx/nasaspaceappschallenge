/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./templates/**/*.html", "./src/**/*.rs"],
  theme: {
    extend: {
      colors: {
        ink: { 950: "#02040d", 900: "#050812", 850: "#090d18", 800: "#101522", 700: "#111725" },
        line: "#2a3040",
        fg: "#f5f5f7",
        muted: "#a5a8b5",
        dim: "#8b8f9d",
        sky: "#2997ff",
        blue: "#0071e3",
        amber: "#e5b67a",
        cyan: "#91c7d4",
        rowhl: "#10203a",
      },
      fontFamily: { sans: ["Inter", "system-ui", "sans-serif"] },
      maxWidth: { page: "1440px" },
    },
  },
};
