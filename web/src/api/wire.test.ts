import { sampleConfig } from "./mock";
import {
  blurToPx,
  docFromWire,
  docToWire,
  historyFromWire,
  settingsFromWire,
  settingsToWire,
  statusEventFromWire,
  statusFromWire,
  toWireId,
} from "./wire";

/** A config document exactly as the Rust server sends it. */
const serverDoc = {
  settings: {
    title: "Lab",
    theme: "dark",
    accent: "emerald",
    background: { url: "/bg.jpg", blur: "sm", opacity: 50, saturate: 120 },
    cardBlur: "md",
    customCss: null,
    search: { provider: "custom", url: "https://s.test/?q={q}" },
  },
  groups: [
    {
      id: 7,
      name: "Media",
      hidden: false,
      layout: { columns: null, style: "column" },
      items: [
        {
          id: 12,
          name: "Sonarr",
          icon: "sonarr.png",
          iconKind: "image",
          href: "http://sonarr:8989",
          check: { kind: "http", target: null },
          widget: {
            type: "sonarr",
            fields: { url: "http://sonarr:8989", key: { $secret: "set" } },
          },
        },
      ],
      groups: [
        {
          id: 8,
          name: "Sub",
          hidden: true,
          layout: { columns: 2, style: "row" },
          items: [],
          groups: [],
        },
      ],
    },
  ],
  bookmarks: [
    {
      id: 3,
      name: "Dev",
      items: [{ id: 4, name: "GitHub", abbr: "GH", href: "https://github.com" }],
    },
  ],
  infoWidgets: [
    { id: 1, type: "clock", options: {} },
    { id: 2, type: "unknown-kind", options: {} },
  ],
  version: 5,
};

describe("wire mapping", () => {
  it("reads the server document", () => {
    const doc = docFromWire(serverDoc);
    expect(doc.version).toBe("5");
    expect(doc.groups[0]!.id).toBe("7");
    expect(doc.groups[0]!.layout).toEqual({ columns: 3, style: "column" });
    expect(doc.groups[0]!.groups![0]!.hidden).toBe(true);
    const svc = doc.groups[0]!.items[0]!;
    expect(svc.id).toBe("12");
    expect(svc.check).toEqual({ kind: "http", target: undefined, expectStatus: undefined });
    expect(svc.widget!.fields.key).toEqual({ $secret: "set" });
    expect(doc.bookmarks[0]!.items[0]!.id).toBe("4");
    expect(doc.infoWidgets.map((w) => w.type)).toEqual(["clock"]);
    expect(doc.settings).toEqual({
      title: "Lab",
      theme: "dark",
      accent: "#10b981",
      background: { url: "/bg.jpg", blur: 4, opacity: 0.5, saturate: 120, brightness: 100 },
      cardBlur: 12,
      customCss: "",
      search: { provider: "custom", url: "https://s.test/?q=%s" },
    });
  });

  it("round-trips server ids and sends null for new items", () => {
    const doc = docFromWire(serverDoc);
    doc.groups[0]!.items.push({ id: "s-abc123", name: "New", check: null, widget: null });
    const wire = docToWire(doc) as {
      groups: { id: unknown; items: { id: unknown }[] }[];
      version: unknown;
    };
    expect(wire.version).toBe(5);
    expect(wire.groups[0]!.id).toBe(7);
    expect(wire.groups[0]!.items.map((s) => s.id)).toEqual([12, null]);
  });

  it("writes settings in the server's shape", () => {
    expect(settingsToWire(settingsFromWire(serverDoc.settings))).toEqual({
      title: "Lab",
      theme: "dark",
      accent: "#10b981",
      background: {
        url: "/bg.jpg",
        blur: "4px",
        opacity: 50,
        saturate: 120,
        brightness: null,
      },
      cardBlur: "12px",
      customCss: null,
      search: { provider: "custom", url: "https://s.test/?q={q}" },
    });
    expect(settingsFromWire(null).accent).toMatch(/^#/);
  });

  it("works on the mock's sample document too", () => {
    const sample = sampleConfig();
    const back = docFromWire(docToWire(sample));
    expect(back.groups.map((g) => g.name)).toEqual(sample.groups.map((g) => g.name));
  });

  it("parses blur sizes", () => {
    expect(blurToPx("md")).toBe(12);
    expect(blurToPx("10px")).toBe(10);
    expect(blurToPx(null)).toBe(0);
  });

  it("parses ids", () => {
    expect(toWireId("42")).toBe(42);
    expect(toWireId("s-1")).toBeNull();
    expect(toWireId(undefined)).toBeNull();
  });

  it("reads history, status and events", () => {
    expect(historyFromWire([{ version: 3, createdAt: 0, author: null, summary: null }])).toEqual([
      { version: "3", createdAt: "1970-01-01T00:00:00.000Z", author: "unknown", summary: "Saved" },
    ]);
    const st = statusFromWire({
      "12": { up: true, latencyMs: 20, checkedAt: 5, history: [{ t: 1, up: false }] },
    });
    expect(st["12"]!.history).toEqual([{ t: 1, up: false, ms: undefined }]);
    expect(statusEventFromWire({ serviceId: 12, up: true, checkedAt: 9 })!.serviceId).toBe("12");
    expect(statusEventFromWire({})).toBeNull();
  });
});

describe("icon names", () => {
  it("resolves like the server", async () => {
    const { resolveIconName } = await import("./client");
    expect(resolveIconName("sonarr").url).toBe(
      "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/png/sonarr.png",
    );
    expect(resolveIconName("sonarr.svg").url).toMatch(/\/svg\/sonarr\.svg$/);
    expect(resolveIconName("sh-immich.webp").url).toBe(
      "https://cdn.jsdelivr.net/gh/selfhst/icons/webp/immich.webp",
    );
    expect(resolveIconName("mdi-server")).toEqual({
      url: "https://cdn.jsdelivr.net/npm/@mdi/svg@7.4.47/svg/server.svg",
      kind: "mask",
    });
    expect(resolveIconName("custom/me.png").url).toBe("/icons/custom/me.png");
    expect(resolveIconName("../etc").url).toBe("");
  });
});
