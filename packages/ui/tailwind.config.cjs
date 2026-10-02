/** Shared Tailwind config: apps extend `content` with their own paths. */
const path = require("node:path");
module.exports = {
  presets: [require("@cia/tokens/tailwind-preset")],
  content: [path.join(__dirname, "src/**/*.{ts,tsx}")],
};
