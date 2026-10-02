/** Placeholder until the WASM engine host lands (M2): falls back to the mock so the shell renders. */
import { MockHost } from "@cia/engine-client/mock";
export class WebHost extends MockHost {
  constructor() {
    super({ kind: "web" });
  }
}
