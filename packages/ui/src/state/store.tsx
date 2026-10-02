import { createContext, useContext, useReducer, type Dispatch, type ReactNode } from "react";
import type { EngineEvent, InputItem, ItemOutcome, ItemState, JobSummary, Plan } from "@cia/engine-client";

export type Destination =
  | { type: "preset"; presetId: string }
  | { type: "custom"; bytes: number; perMessage: boolean }
  | { type: "smaller" };

export interface ItemProgress {
  state: ItemState;
  fraction: number;
  etaMs: number | null;
  label: string;
  outcome?: ItemOutcome;
}

export type Phase = "empty" | "files" | "planning" | "planned" | "running" | "done";

export interface AppState {
  items: InputItem[];
  destination: Destination | null;
  plan: Plan | null;
  planToken: number;
  phase: Phase;
  jobId: string | null;
  progress: Record<string, ItemProgress>;
  summary: JobSummary | null;
  jobStartedAt: number | null;
  drawerOpen: boolean;
  trimItemId: string | null;
  modal: null | "settings" | "licenses" | "upgrade" | "activate" | "ffmpeg" | "help" | "allowance" | "welcome";
  settingsSection: string;
  toast: string | null;
}

export const initialState: AppState = {
  items: [],
  destination: null,
  plan: null,
  planToken: 0,
  phase: "empty",
  jobId: null,
  progress: {},
  summary: null,
  jobStartedAt: null,
  drawerOpen: false,
  trimItemId: null,
  modal: null,
  settingsSection: "general",
  toast: null,
};

export type Action =
  | { type: "add_items"; items: InputItem[] }
  | { type: "remove_item"; id: string }
  | { type: "clear" }
  | { type: "set_destination"; destination: Destination | null }
  | { type: "planning"; token: number }
  | { type: "planned"; plan: Plan; token: number }
  | { type: "plan_failed"; token: number }
  | { type: "job_started"; jobId: string }
  | { type: "engine_event"; event: EngineEvent }
  | { type: "job_finished"; summary: JobSummary }
  | { type: "start_over" }
  | { type: "drawer"; open: boolean }
  | { type: "trim"; itemId: string | null }
  | { type: "modal"; modal: AppState["modal"]; section?: string }
  | { type: "toast"; message: string | null };

function phaseForItems(items: InputItem[], destination: Destination | null): Phase {
  if (items.length === 0) return "empty";
  return destination ? "planning" : "files";
}

export function reducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case "add_items": {
      const items = [...state.items, ...action.items];
      return { ...state, items, plan: null, summary: null, progress: {}, phase: phaseForItems(items, state.destination) };
    }
    case "remove_item": {
      const items = state.items.filter((i) => i.id !== action.id);
      return { ...state, items, plan: null, summary: null, progress: {}, phase: phaseForItems(items, state.destination) };
    }
    case "clear":
      return { ...initialState, destination: state.destination, modal: state.modal };
    case "set_destination":
      return { ...state, destination: action.destination, plan: null, summary: null, progress: {}, phase: phaseForItems(state.items, action.destination) };
    case "planning":
      return { ...state, planToken: action.token, phase: "planning" };
    case "planned":
      if (action.token !== state.planToken) return state;
      return { ...state, plan: action.plan, phase: "planned" };
    case "plan_failed":
      if (action.token !== state.planToken) return state;
      return { ...state, plan: null, phase: "files" };
    case "job_started": {
      const progress: Record<string, ItemProgress> = {};
      for (const i of state.items) progress[i.id] = { state: { type: "queued" }, fraction: 0, etaMs: null, label: "Waiting" };
      return { ...state, jobId: action.jobId, phase: "running", progress, summary: null, jobStartedAt: Date.now(), drawerOpen: false, trimItemId: null };
    }
    case "engine_event": {
      const e = action.event;
      if (e.job_id !== state.jobId) return state;
      switch (e.type) {
        case "item_state":
          return { ...state, progress: { ...state.progress, [e.item_id]: { ...(state.progress[e.item_id] ?? { fraction: 0, etaMs: null, label: "" }), state: e.state, label: labelFor(e.state) } } };
        case "progress":
          return { ...state, progress: { ...state.progress, [e.item_id]: { ...(state.progress[e.item_id] ?? { state: { type: "encoding", attempt: 1 } }), fraction: e.fraction, etaMs: e.eta_ms === null ? null : Number(e.eta_ms), label: e.label } } };
        case "item_done":
          return { ...state, progress: { ...state.progress, [e.item_id]: { ...(state.progress[e.item_id] ?? { fraction: 1, etaMs: null, label: "" }), state: stateFor(e.outcome), fraction: 1, outcome: e.outcome } } };
        case "job_done":
          return { ...state, summary: e.summary, phase: "done" };
        default:
          return state;
      }
    }
    case "job_finished":
      return { ...state, summary: action.summary, phase: "done" };
    case "start_over":
      return { ...initialState, destination: state.destination };
    case "drawer":
      return { ...state, drawerOpen: action.open };
    case "trim":
      return { ...state, trimItemId: action.itemId };
    case "modal":
      return { ...state, modal: action.modal, settingsSection: action.section ?? state.settingsSection };
    case "toast":
      return { ...state, toast: action.message };
  }
}

function labelFor(s: ItemState): string {
  switch (s.type) {
    case "queued": return "Waiting";
    case "inspecting": return "Reading";
    case "planning": return "Planning";
    case "encoding": return s.attempt > 1 ? `Trying again (${s.attempt})` : "Working";
    case "verifying": return "Checking";
    case "retry": return "Trying again";
    case "fitted": return "Done";
    case "kept_original": return "Already fits";
    case "refused": return "Couldn't fit";
    case "failed": return "Failed";
    case "cancelled": return "Stopped";
  }
}

function stateFor(o: ItemOutcome): ItemState {
  switch (o.type) {
    case "fitted": return { type: "fitted" };
    case "kept_original": return { type: "kept_original" };
    case "refused": return { type: "refused" };
    case "failed": return { type: "failed" };
    case "cancelled": return { type: "cancelled" };
  }
}

const StoreCtx = createContext<{ state: AppState; dispatch: Dispatch<Action> } | null>(null);

export function StoreProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(reducer, initialState);
  return <StoreCtx.Provider value={{ state, dispatch }}>{children}</StoreCtx.Provider>;
}

export function useStore() {
  const v = useContext(StoreCtx);
  if (!v) throw new Error("useStore outside StoreProvider");
  return v;
}
