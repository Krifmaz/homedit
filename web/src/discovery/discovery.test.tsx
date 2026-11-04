import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, vi } from "vitest";
import { MockApi } from "../api/mock";
import type { ScanJob } from "../api/types";
import { App } from "../app/App";
import { AppProvider } from "../app/context";
import {
  bareNetworks,
  foundServices,
  parseTargets,
  scanProgress,
  suggestionToService,
  widenBareNetworks,
  widenQuestion,
} from "./suggest";

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

  it("spots network addresses written without a prefix length", () => {
    expect(bareNetworks("10.0.1.0\n10.0.2.0/24\n10.0.0.5\n300.1.1.0")).toEqual(["10.0.1.0"]);
    expect(widenBareNetworks("10.0.1.0\n192.168.5.0, nas.lan\n10.0.2.0/24")).toBe(
      "10.0.1.0/24\n192.168.5.0/24, nas.lan\n10.0.2.0/24",
    );
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

describe("bare network addresses", () => {
  afterEach(() => vi.restoreAllMocks());

  async function openWith(user: ReturnType<typeof userEvent.setup>, text: string) {
    await user.click(await screen.findByRole("button", { name: /Edit/ }));
    await user.click(await screen.findByRole("button", { name: "Find services" }));
    const box = screen.getByLabelText("Networks and hosts");
    await user.clear(box);
    await user.type(box, text);
    return box;
  }

  it("asks about the /24 in plain words", () => {
    expect(widenQuestion(["10.0.1.0"])).toMatch(
      /Scan 10\.0\.1\.0\/24 \(the whole network\) instead\?/,
    );
    expect(widenQuestion(["10.0.1.0", "10.0.2.0"])).toMatch(/^10\.0\.1\.0, 10\.0\.2\.0 are/);
  });

  it("offers the /24 when the scan starts and scans it on yes", async () => {
    const { api, user } = setup();
    const ask = vi.spyOn(window, "confirm").mockReturnValue(true);
    const startScan = vi.spyOn(api, "startScan");
    const box = await openWith(user, "10.0.1.0");
    await user.click(screen.getByRole("button", { name: "Start scan" }));
    expect(ask).toHaveBeenCalledWith(widenQuestion(["10.0.1.0"]));
    await waitFor(() => expect(startScan).toHaveBeenCalled());
    expect(startScan.mock.calls[0]![0].targets).toEqual(["10.0.1.0/24"]);
    expect(box).toHaveValue("10.0.1.0/24");
  });

  it("scans just the address on no", async () => {
    const { api, user } = setup();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    const startScan = vi.spyOn(api, "startScan");
    await openWith(user, "10.0.1.0");
    await user.click(screen.getByRole("button", { name: "Start scan" }));
    await waitFor(() => expect(startScan).toHaveBeenCalled());
    expect(startScan.mock.calls[0]![0].targets).toEqual(["10.0.1.0"]);
  });

  it("offers the /24 again when scanning the bare address found nothing", async () => {
    const { api, user } = setup();
    const empty: ScanJob = {
      id: 1,
      running: false,
      events: [],
      suggestions: {},
      report: {
        targets: ["10.0.1.0"],
        started_at_ms: 0,
        finished_at_ms: 1,
        probes_sent: 120,
        hosts: [],
      },
    };
    const scanJob = vi.spyOn(api, "scanJob").mockResolvedValue(empty);
    const startScan = vi.spyOn(api, "startScan");
    const box = await openWith(user, "10.0.1.0");
    expect(
      await screen.findByText(/Only the single address 10\.0\.1\.0 was scanned/),
    ).toBeInTheDocument();
    scanJob.mockRestore();
    await user.click(screen.getByRole("button", { name: "Scan 10.0.1.0/24" }));
    await waitFor(() => expect(startScan).toHaveBeenCalled());
    expect(startScan.mock.calls[0]![0].targets).toEqual(["10.0.1.0/24"]);
    expect(box).toHaveValue("10.0.1.0/24");
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

describe("rescans", () => {
  it("labels intervals", async () => {
    const { intervalLabel } = await import("./ScanPanel");
    expect(intervalLabel(1)).toBe("hour");
    expect(intervalLabel(6)).toBe("6 hours");
    expect(intervalLabel(24)).toBe("day");
    expect(intervalLabel(72)).toBe("3 days");
    expect(intervalLabel(168)).toBe("week");
    expect(intervalLabel(720)).toBe("30 days");
  });

  it("describes the next run", async () => {
    const { scheduleTimes } = await import("./ScanPanel");
    const base = {
      enabled: true,
      intervalHours: 6,
      request: { targets: ["x"], ports: { kind: "common" as const }, fingerprint: true },
    };
    expect(scheduleTimes({ ...base, nextRunAt: 0 }, 1000)).toBe("Next run within a minute.");
    expect(scheduleTimes({ ...base, nextRunAt: 5000 }, 1000)).toMatch(/^Next run .+\.$/);
    expect(scheduleTimes({ ...base, enabled: false, nextRunAt: null, lastRunAt: 1 }, 1000)).toMatch(
      /^Last run/,
    );
  });

  it("saves a schedule with the form's targets", async () => {
    const { api, user } = setup();
    await user.click(await screen.findByRole("button", { name: /Edit/ }));
    await user.click(await screen.findByRole("button", { name: "Find services" }));
    await user.click(await screen.findByLabelText("Rescan on a schedule"));
    await user.selectOptions(screen.getByLabelText("Every"), "week");
    await user.click(screen.getByRole("button", { name: "Save schedule" }));
    expect(await screen.findByText("Schedule saved.")).toBeInTheDocument();
    const s = await api.scanSchedule();
    expect(s.enabled).toBe(true);
    expect(s.intervalHours).toBe(168);
    expect(s.request.targets).toEqual(["192.168.1.0/24"]);
    expect(screen.getByText(/Next run/)).toBeInTheDocument();
  });

  it("shows what changed on a rescan and raises a notice", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /Edit/ }));
    await user.click(await screen.findByRole("button", { name: "Find services" }));
    await user.click(screen.getByRole("button", { name: "Start scan" }));
    await screen.findByText(/Found 4 open ports/);
    expect(
      screen.queryByRole("region", { name: "Changes since the last scan" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Start scan" }));
    const changes = await screen.findByRole("region", { name: "Changes since the last scan" });
    expect(changes).toHaveTextContent("Home Assistant");
    expect(changes).toHaveTextContent("No longer answers on 192.168.1.30:5432");

    // The panel lists the changes itself; the notice shows once it closes.
    expect(screen.queryByText("Network changes found")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(await screen.findByText("Network changes found")).toBeInTheDocument();
    expect(screen.getByText("New: Home Assistant at 192.168.1.30:8123")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("Network changes found")).not.toBeInTheDocument();
  });
});
