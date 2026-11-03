import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MockApi } from "../api/mock";
import { App } from "./App";
import { AppProvider } from "./context";

function setup(role: "admin" | "viewer" = "admin") {
  const api = new MockApi({ role, liveEvents: false });
  render(
    <AppProvider api={api}>
      <App />
    </AppProvider>,
  );
  return { api, user: userEvent.setup() };
}

describe("App", () => {
  it("renders groups, cards and bookmarks", async () => {
    setup();
    expect(await screen.findByRole("heading", { name: "Media" })).toBeInTheDocument();
    expect(screen.getByText("Sonarr")).toBeInTheDocument();
    expect(screen.getByText("GitHub")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Plex/ })).toHaveAttribute(
      "href",
      "http://plex.lan:32400",
    );
  });

  it("hides hidden groups and the edit button from viewers", async () => {
    setup("viewer");
    await screen.findByRole("heading", { name: "Media" });
    expect(screen.queryByRole("heading", { name: "Admin only" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Edit/ })).not.toBeInTheDocument();
  });

  it("filters cards from the search box", async () => {
    const { user } = setup();
    await screen.findByText("Sonarr");
    await user.type(screen.getByRole("searchbox"), "arr");
    expect(screen.getByText("Sonarr")).toBeInTheDocument();
    expect(screen.queryByText("Plex")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Infrastructure" })).not.toBeInTheDocument();
  });

  it("deletes a card, undoes, redoes and saves", async () => {
    const { api, user } = setup();
    await user.click(await screen.findByRole("button", { name: /Edit/ }));
    const toolbar = await screen.findByRole("toolbar", { name: "Editor" });
    await user.click(screen.getByRole("button", { name: "Delete Plex" }));
    expect(screen.queryByText("Movies and shows")).not.toBeInTheDocument();
    expect(within(toolbar).getByText("Unsaved changes")).toBeInTheDocument();

    await user.click(within(toolbar).getByRole("button", { name: /Undo/ }));
    expect(screen.getByText("Movies and shows")).toBeInTheDocument();
    await user.click(within(toolbar).getByRole("button", { name: /Redo/ }));
    expect(screen.queryByText("Movies and shows")).not.toBeInTheDocument();

    await user.click(within(toolbar).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(within(toolbar).getByText("All changes saved")).toBeInTheDocument());
    const cfg = await api.config();
    expect(cfg.version).toBe("v2");
    expect(cfg.groups[0]!.items.some((s) => s.id === "s-plex")).toBe(false);
  });

  it("supports keyboard undo in edit mode", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /Edit/ }));
    await user.click(await screen.findByRole("button", { name: "Delete Sonarr" }));
    expect(screen.queryByText("TV series")).not.toBeInTheDocument();
    await user.keyboard("{Control>}z{/Control}");
    expect(screen.getByText("TV series")).toBeInTheDocument();
  });
});
