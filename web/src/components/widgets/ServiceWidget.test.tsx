import { render, screen } from "@testing-library/react";
import { MockApi, sampleConfig } from "../../api/mock";
import type { Service } from "../../api/types";
import { AppProvider } from "../../app/context";
import { ServiceWidget } from "./ServiceWidget";

const services = sampleConfig().groups.flatMap((g) => g.items);
const byId = (id: string): Service => services.find((s) => s.id === id)!;

function renderWidget(service: Service, api = new MockApi({ liveEvents: false })) {
  return render(
    <AppProvider api={api}>
      <div data-testid="host">
        <ServiceWidget service={service} />
      </div>
    </AppProvider>,
  );
}

describe("ServiceWidget", () => {
  it("renders stats for the mock Sonarr service", async () => {
    renderWidget(byId("s-sonarr"));
    expect(await screen.findByText("Wanted")).toBeInTheDocument();
    expect(screen.getByText("Queued")).toBeInTheDocument();
    expect(screen.getByText("118")).toBeInTheDocument();
  });

  it("renders nothing when the service has no widget", () => {
    const api = new MockApi({ liveEvents: false });
    const spy = vi.spyOn(api, "widgetData");
    renderWidget(byId("s-jellyfin"), api);
    expect(screen.getByTestId("host")).toBeEmptyDOMElement();
    expect(spy).not.toHaveBeenCalled();
  });

  it("shows at most four stats with units", async () => {
    const api = new MockApi({ liveEvents: false });
    api.widgetData = async () => ({
      fields: [1, 2, 3, 4, 5].map((n) => ({ label: `L${n}`, value: n, unit: "MB/s" })),
    });
    renderWidget(byId("s-sonarr"), api);
    expect(await screen.findByText("L4")).toBeInTheDocument();
    expect(screen.queryByText("L5")).toBeNull();
    expect(screen.getAllByText("MB/s")).toHaveLength(4);
  });

  it("shows the error and does not throw when the API fails", async () => {
    const api = new MockApi({ liveEvents: false });
    api.widgetData = () => Promise.reject(new Error("upstream timeout"));
    renderWidget(byId("s-sonarr"), api);
    expect(await screen.findByText("upstream timeout")).toBeInTheDocument();
  });
});
