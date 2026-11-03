import { sampleConfig } from "../api/mock";
import { filterGroups, parseDragId, serviceMoveFor } from "./Dashboard";
import { initials } from "./Icon";
import { matches, searchUrl } from "./SearchBar";
import { sparkPath } from "./Sparkline";
import { statusText } from "./StatusBadge";

describe("search", () => {
  it("builds provider URLs", () => {
    expect(searchUrl({ provider: "duckduckgo" }, "a b")).toBe("https://duckduckgo.com/?q=a%20b");
    expect(searchUrl({ provider: "google" }, "x")).toBe("https://www.google.com/search?q=x");
    expect(searchUrl({ provider: "custom", url: "https://s.test/?q=%s" }, "a&b")).toBe(
      "https://s.test/?q=a%26b",
    );
    expect(searchUrl({ provider: "custom" }, "x")).toContain("duckduckgo");
  });

  it("matches case-insensitively over any field", () => {
    expect(matches("PLE", "Plex")).toBe(true);
    expect(matches("tv", "Sonarr", "TV series")).toBe(true);
    expect(matches("zzz", "Plex", undefined)).toBe(false);
    expect(matches("  ", "anything")).toBe(true);
  });

  it("filters groups and drops empty ones", () => {
    const out = filterGroups(sampleConfig().groups, "arr");
    expect(out.map((g) => g.id)).toEqual(["g-media"]);
    expect(out[0]!.items.map((s) => s.id)).toEqual(["s-sonarr", "s-radarr"]);
  });
});

describe("drag helpers", () => {
  it("parses drag ids", () => {
    expect(parseDragId("svc:s-1")).toEqual({ kind: "svc:", id: "s-1" });
    expect(parseDragId("group:a:b")).toEqual({ kind: "group:", id: "a:b" });
  });

  it("computes service moves", () => {
    const doc = sampleConfig();
    expect(serviceMoveFor(doc, "s-plex", { kind: "svc:", id: "s-sonarr" }, true)).toEqual({
      groupId: "g-media",
      index: 2,
    });
    expect(serviceMoveFor(doc, "s-plex", { kind: "svc:", id: "s-sonarr" }, false)).toBeNull();
    expect(serviceMoveFor(doc, "s-plex", { kind: "svc:", id: "s-proxmox" }, false)).toEqual({
      groupId: "g-infra",
      index: 0,
    });
    expect(serviceMoveFor(doc, "s-plex", { kind: "drop:", id: "g-infra" }, false)?.groupId).toBe(
      "g-infra",
    );
    expect(serviceMoveFor(doc, "s-plex", { kind: "drop:", id: "g-media" }, false)).toBeNull();
  });
});

describe("cards", () => {
  it("makes initials", () => {
    expect(initials("Home Assistant")).toBe("HA");
    expect(initials("plex")).toBe("PL");
    expect(initials("")).toBe("?");
  });

  it("formats status text", () => {
    expect(statusText({ kind: "http" })).toBe("Checking…");
    expect(
      statusText(
        { kind: "http" },
        { up: true, code: 200, latencyMs: 41.6, checkedAt: 0, history: [] },
      ),
    ).toBe("Up · 200 · 42 ms");
    expect(statusText({ kind: "tcp" }, { up: false, checkedAt: 0, history: [] })).toBe("Down");
  });

  it("breaks the sparkline on down samples", () => {
    const d = sparkPath(
      [
        { t: 0, up: true, ms: 10 },
        { t: 1, up: true, ms: 20 },
        { t: 2, up: false },
        { t: 3, up: true, ms: 10 },
      ],
      30,
      10,
    );
    expect(d.match(/M/g)).toHaveLength(2);
    expect(d.match(/L/g)).toHaveLength(1);
  });
});
