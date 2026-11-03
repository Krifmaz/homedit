import { fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { SettingsForm } from "./SettingsForm";
import { currentDoc, renderForm } from "./testUtils";

describe("SettingsForm", () => {
  it("previews title, theme and accent live", async () => {
    await renderForm(<SettingsForm onClose={() => {}} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Title"), " HQ");
    await user.selectOptions(screen.getByLabelText("Theme"), "dark");
    fireEvent.input(screen.getByLabelText("Accent colour picker"), {
      target: { value: "#ff0000" },
    });
    expect(screen.getByLabelText("Accent colour")).toHaveValue("#ff0000");
    expect(currentDoc().settings).toMatchObject({
      title: "Homelab HQ",
      theme: "dark",
      accent: "#ff0000",
    });
  });

  it("turns the background on and off", async () => {
    await renderForm(<SettingsForm onClose={() => {}} />);
    const user = userEvent.setup();
    const none = screen.getByLabelText("No background");
    expect(none).toBeChecked();
    expect(screen.queryByLabelText("Image URL")).not.toBeInTheDocument();
    await user.click(none);
    await user.type(screen.getByLabelText("Image URL"), "https://img/bg.jpg");
    fireEvent.change(screen.getByLabelText("Blur (px)"), { target: { value: "12" } });
    fireEvent.change(screen.getByLabelText("Opacity"), { target: { value: "0.5" } });
    expect(currentDoc().settings.background).toEqual({
      url: "https://img/bg.jpg",
      blur: 12,
      opacity: 0.5,
    });
    await user.click(none);
    expect(currentDoc().settings.background).toBeNull();
  });

  it("requires %s in a custom search URL", async () => {
    await renderForm(<SettingsForm onClose={() => {}} />);
    const user = userEvent.setup();
    expect(screen.queryByLabelText("Search URL")).not.toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("Search provider"), "custom");
    await user.type(screen.getByLabelText("Search URL"), "https://s.example/?q=");
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
    // "%" is not special for user-event; type it literally.
    await user.type(screen.getByLabelText("Search URL"), "%s");
    expect(screen.getByRole("button", { name: "Done" })).toBeEnabled();
    expect(currentDoc().settings.search).toEqual({
      provider: "custom",
      url: "https://s.example/?q=%s",
    });
    await user.selectOptions(screen.getByLabelText("Search provider"), "bing");
    expect(currentDoc().settings.search).toEqual({ provider: "bing", url: undefined });
  });

  it("edits custom CSS and reverts on Cancel", async () => {
    await renderForm(<SettingsForm onClose={() => {}} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Custom CSS"), "body{{color:red}");
    expect(currentDoc().settings.customCss).toBe("body{color:red}");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(currentDoc().settings.customCss).toBe("");
  });
});
