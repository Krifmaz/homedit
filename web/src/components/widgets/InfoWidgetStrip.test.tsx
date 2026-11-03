import { render, screen } from "@testing-library/react";
import { MockApi, sampleConfig } from "../../api/mock";
import type { InfoWidget } from "../../api/types";
import { AppProvider } from "../../app/context";
import { InfoWidgetStrip } from "./InfoWidgetStrip";

function renderStrip(widgets: InfoWidget[], api = new MockApi({ liveEvents: false })) {
  return render(
    <AppProvider api={api}>
      <InfoWidgetStrip widgets={widgets} />
    </AppProvider>,
  );
}

describe("InfoWidgetStrip", () => {
  it("renders the clock and resources from the mock API", async () => {
    const { container } = renderStrip(sampleConfig().infoWidgets);
    expect(container.querySelector("time.info-clock-time")).not.toBeNull();
    expect(await screen.findByText("6.2 GB / 16 GB")).toBeInTheDocument();
    expect(screen.getByText("1.1 TB / 4 TB")).toBeInTheDocument();
    expect(screen.getByRole("meter", { name: "CPU" })).toBeInTheDocument();
    expect(await screen.findByText("14°C")).toBeInTheDocument();
    expect(await screen.findByText("23")).toBeInTheDocument();
  });

  it("shows °F for imperial units", async () => {
    renderStrip([{ id: "iw-weather", type: "weather", options: { units: "imperial" } }]);
    expect(await screen.findByText("57°F")).toBeInTheDocument();
  });

  it("shows a compact error when the API fails", async () => {
    const api = new MockApi({ liveEvents: false });
    api.infoData = () => Promise.reject(new Error("boom"));
    renderStrip([{ id: "iw-docker", type: "docker", options: {} }], api);
    const err = await screen.findByText("Docker unavailable");
    expect(err).toHaveAttribute("title", "boom");
  });

  it("renders nothing without widgets", () => {
    const { container } = renderStrip([]);
    expect(container.querySelector(".info-strip")).toBeNull();
  });
});
