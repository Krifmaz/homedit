import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MockApi } from "../api/mock";
import type { ScanJob } from "../api/types";
import { App } from "../app/App";
import { AppProvider } from "../app/context";
import { foundServices, parseTargets, scanProgress, suggestionToService } from "./suggest";

const job: ScanJob = {
  id: 1,
  running: false,
  events: [{ event: "progress", done: 50, total: 200, open: 1 }],
  suggestions: {
    "10.0.0.5:8989": {
      name: "Sonarr",
      href: "http://nas:8989/",
      icon: "sonarr.png",
      description: "v4",
      widget: { type: "sonarr", url: "http://nas:8989", nested: { x: 1 } },
      app_id: "sonarr",
    },
  },
  report: {
    targets: ["10.0.0.0/24"],
    started_at_ms: 0,
    finished_at_ms: 1,
    probes_sent: 200,
    hosts: [
      {
        ip: "10.0.0.9",
        hostname: null,
        services: [{ port: 22, source: "scan", http: null, app: null, on_dashboard: false }],
      },
      {
        ip: "10.0.0.5",
        hostname: "nas",
        services: [
          {
            port: 8989,
            source: "scan",
            http: { scheme: "http", status: 200, title: "Sonarr" },
            app: { id: "sonarr", name: "Sonarr", widget: "sonarr", icon: "sonarr", confidence: 90 },
            on_dashboard: false,
          },
        ],
      },
    ],
  },
};

describe("scan helpers", () => {
  it("lists recognised apps first", () => {
    const list = foundServices(job);
    expect(list.map((f) => f.key)).toEqual(["10.0.0.5:8989", "10.0.0.9:22"]);
    expect(list[0]!.host).toBe("nas");
    expect(list[1]!.suggestion).toBeUndefined();
  });

  it("builds a card from a suggestion, keeping only plain widget fields", () => {
    const svc = suggestionToService(job.suggestions["10.0.0.5:8989"]!, "s-1");
    expect(svc).toEqual({
      id: "s-1",
      name: "Sonarr",
      href: "http://nas:8989/",
      icon: "sonarr.png",
      description: "v4",
      check: { kind: "http" },
      widget: { type: "sonarr", fields: { url: "http://nas:8989" } },
    });
  });

  it("reports progress and parses targets", () => {
    expect(scanProgress(job)).toBe(0.25);
    expect(scanProgress({ ...job, events: [] })).toBeNull();
    expect(parseTargets("192.168.1.0/24\n 10.0.0.5 , nas.lan\n\n")).toEqual([
      "192.168.1.0/24",
      "10.0.0.5",
      "nas.lan",
    ]);
  });
});

function setup() {
  const api = new MockApi({ liveEvents: false });
  render(
    <AppProvider api={api}>
      <App />
    </AppProvider>,
  );
  return { api, user: userEvent.setup() };
}

describe("scan panel", () => {
  it("scans and adds a found service to the dashboard", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /Edit/ }));
    await user.click(await screen.findByRole("button", { name: "Find services" }));
    await user.click(screen.getByRole("button", { name: "Start scan" }));
    expect(await screen.findByText(/Found 4 open ports/)).toBeInTheDocument();
    // Sonarr is already on the dashboard and hidden by default.
    expect(screen.queryByRole("button", { name: /Add Sonarr/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Add Jellyfin on media.lan:8096/ }));
    // The card's form opens with the URL and widget type filled in.
    expect(await screen.findByRole("heading", { name: "Edit service" })).toBeInTheDocument();
    expect(screen.getByDisplayValue("http://media.lan:8096/")).toBeInTheDocument();
  });

  it("refuses a public target", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /Edit/ }));
    await user.click(await screen.findByRole("button", { name: "Find services" }));
    const box = screen.getByLabelText("Networks and hosts");
    await user.clear(box);
    await user.type(box, "8.8.8.8");
    await user.click(screen.getByRole("button", { name: "Start scan" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("not a private address");
  });
});

describe("backup panel", () => {
  it("imports files and shows warnings", async () => {
    const { api, user } = setup();
    await user.click(await screen.findByRole("button", { name: /Edit/ }));
    await user.click(await screen.findByRole("button", { name: "Import / export" }));
    const files = [
      new File(["- Media: []"], "services.yaml", { type: "text/yaml" }),
      new File(["x"], "notes.yml", { type: "text/yaml" }),
    ];
    await user.upload(screen.getByLabelText("Files"), files);
    await user.click(screen.getByRole("button", { name: "Import" }));
    expect(await screen.findByText(/Imported as version v2/)).toBeInTheDocument();
    expect(screen.getByText(/Unknown file name/)).toBeInTheDocument();
    await waitFor(async () => expect((await api.history())[0]!.summary).toMatch(/services.yaml/));
    expect(screen.getByRole("link", { name: "Download (without secrets)" })).toHaveAttribute(
      "href",
      "/api/export?secrets=false",
    );
  });
});
