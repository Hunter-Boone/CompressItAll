const base = require("@cia/ui/tailwind.config");
module.exports = { ...base, content: [...base.content, "./index.html", "./src/**/*.{ts,tsx}"] };
