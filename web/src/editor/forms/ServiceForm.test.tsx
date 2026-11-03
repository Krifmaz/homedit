import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MockApi } from "../../api/mock";
import { findService, findServiceGroup } from "../ops";
import { ServiceForm } from "./ServiceForm";
import { currentDoc, renderForm } from "./testUtils";

afterEach(() => vi.restoreAllMocks());

describe("ServiceForm", () => {
  it("edits the name live and keeps it on Done", async () => {
    const onClose = vi.fn();
    await renderForm(<ServiceForm groupId="g-media" serviceId="s-plex" onClose={onClose} />);
    const user = userEvent.setup();
    const name = screen.getByLabelText("Name");
    expect(name).toHaveValue("Plex");
    await user.clear(name);
    await user.type(name, "Plex Server");
    expect(findService(currentDoc(), "s-plex")?.name).toBe("Plex Server");
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(onClose).toHaveBeenCalled();
    expect(findService(currentDoc(), "s-plex")?.name).toBe("Plex Server");
  });

  it("reverts every change of the session on Cancel", async () => {
    const onClose = vi.fn();
    await renderForm(<ServiceForm groupId="g-media" serviceId="s-plex" onClose={onClose} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Name"), " 2");
    await user.type(screen.getByLabelText("Description"), "!");
    expect(findService(currentDoc(), "s-plex")?.name).toBe("Plex 2");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onClose).toHaveBeenCalled();
    const svc = findService(currentDoc(), "s-plex");
    expect(svc?.name).toBe("Plex");
    expect(svc?.description).toBe("Movies and shows");
  });

  it("disables Done while the name is missing and shows the error", async () => {
    await renderForm(<ServiceForm groupId="g-media" serviceId="s-plex" onClose={() => {}} />);
    const user = userEvent.setup();
    await user.clear(screen.getByLabelText("Name"));
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
    expect(screen.getByText("Name is required")).toBeInTheDocument();
  });

  it("adds a new service on first change, once", async () => {
    await renderForm(<ServiceForm groupId="g-infra" onClose={() => {}} />);
    const before = currentDoc().groups.find((g) => g.id === "g-infra")!.items.length;
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Delete" })).not.toBeInTheDocument();
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Name"), "Grafana");
    const items = currentDoc().groups.find((g) => g.id === "g-infra")!.items;
    expect(items).toHaveLength(before + 1);
    expect(items.at(-1)?.name).toBe("Grafana");
    expect(items.at(-1)?.id).toMatch(/^s-/);
    expect(screen.getByRole("button", { name: "Done" })).toBeEnabled();
  });

  it("moves the card when the group changes", async () => {
    await renderForm(<ServiceForm groupId="g-media" serviceId="s-sonarr" onClose={() => {}} />);
    const user = userEvent.setup();
    await user.selectOptions(screen.getByLabelText("Group"), "g-infra");
    expect(findServiceGroup(currentDoc(), "s-sonarr")?.id).toBe("g-infra");
  });

  it("configures an HTTP check with an expected status", async () => {
    await renderForm(<ServiceForm groupId="g-media" serviceId="s-jellyfin" onClose={() => {}} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Expected status"), "204");
    expect(findService(currentDoc(), "s-jellyfin")?.check).toEqual({
      kind: "http",
      expectStatus: 204,
    });
    await user.selectOptions(screen.getByLabelText("Check"), "tcp");
    expect(screen.queryByLabelText("Expected status")).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("Target"), "jf:8096");
    expect(findService(currentDoc(), "s-jellyfin")?.check).toEqual({
      kind: "tcp",
      target: "jf:8096",
    });
    await user.selectOptions(screen.getByLabelText("Check"), "");
    expect(findService(currentDoc(), "s-jellyfin")?.check).toBeNull();
  });

  it("picks a widget, fills its form and tests the connection", async () => {
    const api = new MockApi({ liveEvents: false });
    const spy = vi.spyOn(api, "testWidget");
    await renderForm(
      <ServiceForm groupId="g-media" serviceId="s-jellyfin" onClose={() => {}} />,
      api,
    );
    const user = userEvent.setup();
    const picker = screen.getByLabelText("Widget type");
    expect(within(picker).getByRole("group", { name: "Media servers" })).toBeInTheDocument();
    await user.selectOptions(picker, "jellyfin");
    // URL is pre-filled from the link.
    expect(screen.getByLabelText("URL")).toHaveValue("http://jellyfin.lan:8096");
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
    await user.type(screen.getByLabelText("API key"), "abc");
    expect(findService(currentDoc(), "s-jellyfin")?.widget).toEqual({
      type: "jellyfin",
      fields: { url: "http://jellyfin.lan:8096", key: "abc", enableNowPlaying: true },
    });
    expect(screen.getByRole("button", { name: "Done" })).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "Test connection" }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Connected to jellyfin at http://jellyfin.lan:8096",
    );
    expect(spy).toHaveBeenCalledWith(
      { type: "jellyfin", fields: expect.objectContaining({ key: "abc" }) },
      "s-jellyfin",
    );

    await user.clear(screen.getByLabelText("URL"));
    await user.type(screen.getByLabelText("URL"), "jellyfin");
    expect(screen.getByText("URL must start with http:// or https://")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(/✗/));
  });

  it("keeps stored secrets as placeholders", async () => {
    await renderForm(<ServiceForm groupId="g-media" serviceId="s-plex" onClose={() => {}} />);
    expect(screen.getByLabelText("Plex token")).toHaveAttribute("placeholder", "••••••••");
    expect(screen.getByRole("button", { name: "Done" })).toBeEnabled();
  });

  it("deletes after confirmation", async () => {
    const onClose = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    await renderForm(<ServiceForm groupId="g-media" serviceId="s-radarr" onClose={onClose} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Delete" }));
    expect(confirm).toHaveBeenCalled();
    expect(findService(currentDoc(), "s-radarr")).toBeUndefined();
    expect(onClose).toHaveBeenCalled();
  });

  it("does nothing when deletion is not confirmed", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(false);
    await renderForm(<ServiceForm groupId="g-media" serviceId="s-radarr" onClose={() => {}} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Delete" }));
    expect(findService(currentDoc(), "s-radarr")).toBeDefined();
  });
});
