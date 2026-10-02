import type { SpotlightStep } from "../components/SpotlightTutorial";

export function tourSteps(web: boolean): SpotlightStep[] {
  const here = web ? "this browser" : "this computer";
  return [
    { id: "welcome", title: "Welcome to Smidge", body: "Smidge makes files small enough to send. This tour takes about 20 seconds." },
    { id: "add", target: "[data-tour=dropzone]", title: "1. Add your files", body: `Drag files or a whole folder here, or click to choose them. Your files never leave ${here}.` },
    { id: "pick", target: "[data-tour=destinations]", title: "2. Pick where it's going", body: "Each button knows that app's size limit: Discord, email, WhatsApp and more. If you know your own limit, use Custom size." },
    { id: "smaller", target: "[data-tour=smaller]", title: "No limit in mind?", body: "Just make it smaller shrinks everything as much as it can without visible loss." },
    { id: "check", target: "[data-tour=prediction]", title: "3. Check, then compress", body: "Before it starts, Smidge tells you the size and quality you'll get. If something can't fit, it says so here and suggests a fix." },
    { id: "advanced", target: "[data-tour=advanced]", title: "Extra options", body: "Photo size, video sound, formats and where files are saved live here. You never have to open it." },
    { id: "ready", title: "You're ready", body: "When it's done, copy the file or drag it straight into Discord, WhatsApp or your email." },
  ];
}
