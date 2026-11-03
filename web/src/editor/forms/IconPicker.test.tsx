import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { MockApi } from "../../api/mock";
import { IconPicker } from "./IconPicker";
import { renderForm } from "./testUtils";

/** Holds the picker value and shows it in a heading (which renderForm waits for). */
function Harness({ initial }: { initial?: string }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <h2>Picker</h2>
      <output data-testid="value">{value ?? "(none)"}</output>
      <IconPicker value={value} onChange={setValue} name="Plex" />
    </>
  );
}

describe("IconPicker", () => {
  it("edits the reference directly and previews it", async () => {
    await renderForm(<Harness />);
    const user = userEvent.setup();
    expect(screen.getByTestId("icon-preview").querySelector("img")).toBeNull();
    await user.type(screen.getByLabelText("Icon"), "https://x/icon.png");
    expect(screen.getByTestId("value")).toHaveTextContent("https://x/icon.png");
    expect(screen.getByTestId("icon-preview").querySelector("img")).toHaveAttribute(
      "src",
      "https://x/icon.png",
    );
    await user.clear(screen.getByLabelText("Icon"));
    expect(screen.getByTestId("value")).toHaveTextContent("(none)");
  });

  it("searches after a pause and picks a result", async () => {
    const api = new MockApi({ liveEvents: false });
    const spy = vi.spyOn(api, "searchIcons");
    await renderForm(<Harness />, api);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Search icons"), "sonarr");
    const choice = await screen.findByRole("button", { name: "Use icon sh-sonarr" });
    // Debounced: one request for the final query, not one per keystroke.
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith("sonarr");
    await user.click(choice);
    expect(screen.getByTestId("value")).toHaveTextContent("sh-sonarr");
    expect(choice).toHaveAttribute("aria-pressed", "true");
  });

  it("says when nothing matches", async () => {
    await renderForm(<Harness />);
    await userEvent.setup().type(screen.getByLabelText("Search icons"), "zzzz");
    expect(await screen.findByText("No icons found.")).toBeInTheDocument();
  });

  it("uploads a custom icon and uses its name", async () => {
    const api = new MockApi({ liveEvents: false });
    vi.spyOn(api, "uploadIcon").mockResolvedValue({
      name: "custom:logo.png",
      source: "custom",
      url: "/icons/custom/logo.png",
    });
    await renderForm(<Harness initial="plex.png" />, api);
    const file = new File(["x"], "logo.png", { type: "image/png" });
    await userEvent.setup().upload(screen.getByLabelText("Upload icon file"), file);
    await waitFor(() => expect(screen.getByTestId("value")).toHaveTextContent("custom:logo.png"));
    expect(api.uploadIcon).toHaveBeenCalledWith(file);
  });
});
