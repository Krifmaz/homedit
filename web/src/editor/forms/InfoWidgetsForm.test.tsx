import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { InfoWidgetsForm } from "./InfoWidgetsForm";
import { currentDoc, renderForm } from "./testUtils";

const types = () => currentDoc().infoWidgets.map((w) => w.type);

describe("InfoWidgetsForm", () => {
  it("lists, reorders and removes widgets", async () => {
    await renderForm(<InfoWidgetsForm onClose={() => {}} />);
    const user = userEvent.setup();
    expect(types()).toEqual(["clock", "resources", "weather", "docker"]);
    expect(screen.getByRole("button", { name: "Move Clock 1 up" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Move Clock 1 down" }));
    expect(types()).toEqual(["resources", "clock", "weather", "docker"]);
    await user.click(screen.getByRole("button", { name: "Move Docker 1 up" }));
    expect(types()).toEqual(["resources", "clock", "docker", "weather"]);
    await user.click(screen.getByRole("button", { name: "Remove Host resources 1" }));
    expect(types()).toEqual(["clock", "docker", "weather"]);
  });

  it("adds a widget with default options", async () => {
    await renderForm(<InfoWidgetsForm onClose={() => {}} />);
    const user = userEvent.setup();
    await user.selectOptions(screen.getByLabelText("Add widget"), "weather");
    await user.click(screen.getByRole("button", { name: "Add" }));
    const added = currentDoc().infoWidgets.at(-1)!;
    expect(added).toMatchObject({ type: "weather", options: { units: "metric" } });
    expect(added.id).toMatch(/^iw-/);
    expect(screen.getByRole("button", { name: "Remove Weather 2" })).toBeInTheDocument();
  });

  it("edits per-type options", async () => {
    await renderForm(<InfoWidgetsForm onClose={() => {}} />);
    const user = userEvent.setup();
    await user.selectOptions(screen.getByLabelText("Time format"), "12h");
    await user.type(screen.getByLabelText("Latitude"), "52.5");
    await user.selectOptions(screen.getByLabelText("Units"), "imperial");
    await user.type(screen.getByLabelText("Disk path"), "/mnt/data");
    await user.type(screen.getByLabelText("Docker host"), "tcp://dock:2375");
    const byType = Object.fromEntries(currentDoc().infoWidgets.map((w) => [w.type, w.options]));
    expect(byType.clock).toEqual({ format: "12h" });
    expect(byType.weather).toEqual({ location: "Home", latitude: 52.5, units: "imperial" });
    expect(byType.resources).toEqual({ disk: "/mnt/data" });
    expect(byType.docker).toEqual({ host: "tcp://dock:2375" });
  });

  it("reverts on Cancel", async () => {
    await renderForm(<InfoWidgetsForm onClose={() => {}} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Remove Clock 1" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(types()).toEqual(["clock", "resources", "weather", "docker"]);
  });
});
