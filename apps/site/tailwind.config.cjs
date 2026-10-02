/** Site Tailwind config: the shared token preset plus this app's content paths. */
module.exports = {
  presets: [require("@cia/tokens/tailwind-preset")],
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}"],
};
