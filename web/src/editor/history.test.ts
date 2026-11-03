import { sampleConfig } from "../api/mock";
import { initUndo, isDirty, MAX_UNDO, undoReducer, type UndoState } from "./history";
import { updateSettings } from "./ops";

const title = (s: UndoState) => s.present.settings.title;

describe("undoReducer", () => {
  it("undoes and redoes", () => {
    let s = initUndo(sampleConfig());
    s = undoReducer(s, { type: "apply", doc: updateSettings(s.present, { title: "A" }) });
    s = undoReducer(s, { type: "apply", doc: updateSettings(s.present, { title: "B" }) });
    expect(title(s)).toBe("B");
    s = undoReducer(s, { type: "undo" });
    expect(title(s)).toBe("A");
    s = undoReducer(s, { type: "undo" });
    expect(title(s)).toBe("Homelab");
    s = undoReducer(s, { type: "undo" });
    expect(title(s)).toBe("Homelab");
    s = undoReducer(s, { type: "redo" });
    expect(title(s)).toBe("A");
  });

  it("drops the redo stack on a new edit", () => {
    let s = initUndo(sampleConfig());
    s = undoReducer(s, { type: "apply", doc: updateSettings(s.present, { title: "A" }) });
    s = undoReducer(s, { type: "undo" });
    s = undoReducer(s, { type: "apply", doc: updateSettings(s.present, { title: "C" }) });
    expect(s.future).toHaveLength(0);
  });

  it("coalesces edits with the same key into one step", () => {
    let s = initUndo(sampleConfig());
    for (const t of ["H", "He", "Hel"]) {
      s = undoReducer(s, {
        type: "apply",
        doc: updateSettings(s.present, { title: t }),
        coalesceKey: "k",
      });
    }
    expect(s.past).toHaveLength(1);
    s = undoReducer(s, { type: "undo" });
    expect(title(s)).toBe("Homelab");
  });

  it("caps the undo stack", () => {
    let s = initUndo(sampleConfig());
    for (let i = 0; i < MAX_UNDO + 20; i++) {
      s = undoReducer(s, { type: "apply", doc: updateSettings(s.present, { title: String(i) }) });
    }
    expect(s.past).toHaveLength(MAX_UNDO);
  });

  it("tracks dirty state and stamps the saved version", () => {
    let s = initUndo(sampleConfig());
    expect(isDirty(s)).toBe(false);
    s = undoReducer(s, { type: "apply", doc: updateSettings(s.present, { title: "A" }) });
    expect(isDirty(s)).toBe(true);
    s = undoReducer(s, { type: "saved", version: "v9" });
    expect(isDirty(s)).toBe(false);
    expect(s.present.version).toBe("v9");
    expect(s.past.every((d) => d.version === "v9")).toBe(true);
    s = undoReducer(s, { type: "undo" });
    expect(isDirty(s)).toBe(true);
  });
});

describe("synced", () => {
  it("replaces the present with the server copy and keeps undo", () => {
    let s = initUndo(sampleConfig());
    s = undoReducer(s, { type: "apply", doc: updateSettings(s.present, { title: "A" }) });
    s = undoReducer(s, { type: "saved", version: "v2" });
    const server = { ...s.present, groups: [] };
    s = undoReducer(s, { type: "synced", doc: server });
    expect(s.present).toBe(server);
    expect(isDirty(s)).toBe(false);
    expect(s.past).toHaveLength(1);
  });
});
