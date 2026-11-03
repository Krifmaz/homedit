import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MockApi, sampleConfig } from "../api/mock";
import type { ConfigDoc } from "../api/types";
import { AppProvider } from "../app/context";
import { diffSummary, isEmptyDiff, relativeTime, HistoryPanel } from "./HistoryPanel";

describe("diffSummary", () => {
  it("reports no change for identical documents", () => {
    const d = diffSummary(sampleConfig(), { ...sampleConfig(), version: "other" });
    expect(isEmptyDiff(d)).toBe(true);
  });

  it("counts added, removed and changed items", () => {
    const a = sampleConfig();
    const b: ConfigDoc = structuredClone(a);
    const media = b.groups[0]!;
    media.items[0]!.name = "Plex Media Server"; // changed
    media.items.splice(1, 1); // removed jellyfin (also reorders the group)
    b.groups[1]!.items.push({ id: "s-new", name: "New", check: null, widget: null }); // added
    b.groups.splice(2, 1); // removed group (and its service)
    b.groups.push({
      id: "g-new",
      name: "New group",
      hidden: false,
      layout: { columns: 2, style: "grid" },
      items: [],
    });
    b.bookmarks[0]!.items[0]!.href = "https://github.com/x"; // changed
    b.bookmarks[1]!.items = []; // removed
    b.settings.title = "Other";

    const d = diffSummary(a, b);
    expect(d.services).toEqual({ added: 1, removed: 2, changed: 1 });
    // media (order) and infra (order) changed, admin removed, new added
    expect(d.groups).toEqual({ added: 1, removed: 1, changed: 2 });
    expect(d.bookmarks).toEqual({ added: 0, removed: 1, changed: 1 });
    expect(d.settings).toBe(true);
    expect(d.infoWidgets).toBe(false);
  });

  it("counts a service moved between groups as changed", () => {
    const a = sampleConfig();
    const b = structuredClone(a);
    const moved = b.groups[0]!.items.pop()!;
    b.groups[1]!.items.unshift(moved);
    expect(diffSummary(a, b).services).toEqual({ added: 0, removed: 0, changed: 1 });
  });

  it("includes nested sub-groups and ignores key order", () => {
    const a = sampleConfig();
    a.groups[0]!.groups = [
      { id: "g-sub", name: "Sub", hidden: false, layout: { columns: 1, style: "grid" }, items: [] },
    ];
    const b = structuredClone(a);
    b.groups[0]!.groups![0]!.layout = { style: "grid", columns: 1 };
    expect(isEmptyDiff(diffSummary(a, b))).toBe(true);
    b.groups[0]!.groups![0]!.name = "Renamed";
    expect(diffSummary(a, b).groups).toEqual({ added: 0, removed: 0, changed: 1 });
  });
});

describe("relativeTime", () => {
  const now = Date.parse("2026-01-10T12:00:00Z");
  it("formats recent times", () => {
    expect(relativeTime("2026-01-10T11:59:50Z", now)).toBe("just now");
    expect(relativeTime("2026-01-10T11:55:00Z", now)).toBe("5 min ago");
    expect(relativeTime("2026-01-10T09:00:00Z", now)).toBe("3 h ago");
    expect(relativeTime("2026-01-09T12:00:00Z", now)).toBe("yesterday");
    expect(relativeTime("2026-01-07T12:00:00Z", now)).toBe("3 days ago");
    expect(relativeTime("not a date", now)).toBe("not a date");
  });
});

describe("HistoryPanel", () => {
  async function setup() {
    const api = new MockApi({ liveEvents: false });
    const edited = sampleConfig();
    edited.groups[0]!.items.pop();
    await api.saveConfig(edited); // v2, current
    const onClose = vi.fn();
    render(
      <AppProvider api={api}>
        <HistoryPanel onClose={onClose} />
      </AppProvider>,
    );
    return { api, onClose };
  }

  afterEach(() => vi.restoreAllMocks());

  it("lists entries newest first with the current one not restorable", async () => {
    const { onClose } = await setup();
    expect(screen.getByRole("heading", { name: "Version history" })).toBeInTheDocument();
    const current = await screen.findByTestId("history-v2");
    expect(within(current).getByText("Edited in dashboard")).toBeInTheDocument();
    expect(within(current).getByRole("button", { name: "Restore" })).toBeDisabled();
    const old = screen.getByTestId("history-v1");
    expect(within(old).getByText("Imported from gethomepage")).toBeInTheDocument();
    expect(within(old).getByText("yesterday")).toBeInTheDocument();
    expect(within(old).getByRole("button", { name: "Restore" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalled();
  });

  it("shows a diff summary for the selected entry", async () => {
    await setup();
    const old = await screen.findByTestId("history-v1");
    // wait until the dashboard (current doc) is loaded
    await waitFor(() => expect(screen.queryByText("Loading…")).toBeNull());
    await userEvent.click(within(old).getByText("Imported from gethomepage"));
    expect(await within(old).findByText(/1 added/)).toBeInTheDocument();
  });

  it("restores a version after confirmation and refreshes the list", async () => {
    const { api } = await setup();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const restore = vi.spyOn(api, "restore");
    const old = await screen.findByTestId("history-v1");
    await userEvent.click(within(old).getByRole("button", { name: "Restore" }));
    expect(confirm).toHaveBeenCalled();
    expect(restore).toHaveBeenCalledWith("v1");
    expect(await screen.findByText("Restored v1")).toBeInTheDocument();
  });

  it("does nothing when the confirmation is declined", async () => {
    const { api } = await setup();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    const restore = vi.spyOn(api, "restore");
    const old = await screen.findByTestId("history-v1");
    await userEvent.click(within(old).getByRole("button", { name: "Restore" }));
    expect(restore).not.toHaveBeenCalled();
  });
});
