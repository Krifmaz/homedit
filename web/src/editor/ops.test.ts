import { sampleConfig } from "../api/mock";
import type { ConfigDoc } from "../api/types";
import {
  allGroups,
  deleteBookmark,
  deleteGroup,
  deleteService,
  findService,
  findServiceGroup,
  moveBookmark,
  moveGroup,
  moveService,
  updateSettings,
  upsertBookmark,
  upsertGroup,
  upsertService,
} from "./ops";

const ids = (doc: ConfigDoc, groupId: string) =>
  allGroups(doc)
    .find((g) => g.id === groupId)!
    .items.map((s) => s.id);

describe("ops", () => {
  it("never mutates the input document", () => {
    const doc = sampleConfig();
    const before = JSON.stringify(doc);
    moveService(doc, "s-plex", "g-infra", 0);
    deleteGroup(doc, "g-media");
    updateSettings(doc, { title: "x" });
    expect(JSON.stringify(doc)).toBe(before);
  });

  it("reorders a service within its group", () => {
    const next = moveService(sampleConfig(), "s-plex", "g-media", 2);
    expect(ids(next, "g-media")).toEqual(["s-jellyfin", "s-sonarr", "s-plex", "s-radarr"]);
  });

  it("moves a service to another group and clamps the index", () => {
    const next = moveService(sampleConfig(), "s-plex", "g-infra", 99);
    expect(ids(next, "g-infra")).toEqual(["s-proxmox", "s-pihole", "s-plex"]);
    expect(ids(next, "g-media")).not.toContain("s-plex");
  });

  it("moves groups", () => {
    const next = moveGroup(sampleConfig(), "g-admin", 0);
    expect(next.groups.map((g) => g.id)).toEqual(["g-admin", "g-media", "g-infra"]);
  });

  it("handles nested groups", () => {
    let doc = upsertGroup(
      sampleConfig(),
      { id: "g-sub", name: "Sub", hidden: false, layout: { columns: 2, style: "grid" } },
      "g-media",
    );
    expect(doc.groups.find((g) => g.id === "g-media")!.groups!.map((g) => g.id)).toEqual(["g-sub"]);
    doc = moveService(doc, "s-plex", "g-sub", 0);
    expect(findServiceGroup(doc, "s-plex")!.id).toBe("g-sub");
    doc = deleteGroup(doc, "g-sub");
    expect(findService(doc, "s-plex")).toBeUndefined();
  });

  it("upserts and deletes services", () => {
    const svc = { id: "s-new", name: "New", check: null, widget: null };
    let doc = upsertService(sampleConfig(), "g-infra", svc);
    expect(ids(doc, "g-infra")).toContain("s-new");
    doc = upsertService(doc, "g-infra", { ...svc, name: "Renamed" });
    expect(findService(doc, "s-new")!.name).toBe("Renamed");
    expect(ids(doc, "g-infra").filter((i) => i === "s-new")).toHaveLength(1);
    doc = upsertService(doc, "g-media", svc);
    expect(findServiceGroup(doc, "s-new")!.id).toBe("g-media");
    doc = deleteService(doc, "s-new");
    expect(findService(doc, "s-new")).toBeUndefined();
  });

  it("updates a group without losing its cards", () => {
    const doc = upsertGroup(sampleConfig(), {
      id: "g-media",
      name: "Films",
      hidden: true,
      layout: { columns: 4, style: "row" },
    });
    const g = doc.groups.find((x) => x.id === "g-media")!;
    expect(g.name).toBe("Films");
    expect(g.items).toHaveLength(4);
  });

  it("edits bookmarks", () => {
    let doc = upsertBookmark(sampleConfig(), "b-dev", {
      id: "b-x",
      name: "X",
      href: "https://x.test",
    });
    expect(doc.bookmarks[0]!.items.map((b) => b.id)).toEqual(["b-gh", "b-docs", "b-x"]);
    doc = moveBookmark(doc, "b-x", "b-social", 0);
    expect(doc.bookmarks[1]!.items[0]!.id).toBe("b-x");
    doc = deleteBookmark(doc, "b-x");
    expect(doc.bookmarks.flatMap((g) => g.items).some((b) => b.id === "b-x")).toBe(false);
  });
});
