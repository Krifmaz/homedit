import { fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
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
    const show = screen.getByLabelText("Show a background image");
    expect(show).not.toBeChecked();
    expect(screen.queryByLabelText("Image URL")).not.toBeInTheDocument();
    await user.click(show);
    await user.type(screen.getByLabelText("Image URL"), "https://img/bg.jpg");
    fireEvent.change(screen.getByLabelText("Blur (px)"), { target: { value: "12" } });
    fireEvent.change(screen.getByLabelText("Opacity"), { target: { value: "0.5" } });
    fireEvent.change(screen.getByLabelText("Saturation"), { target: { value: "150" } });
    fireEvent.change(screen.getByLabelText("Brightness"), { target: { value: "70" } });
    expect(currentDoc().settings.background).toEqual({
      url: "https://img/bg.jpg",
      blur: 12,
      opacity: 0.5,
      saturate: 150,
      brightness: 70,
    });
    expect(screen.getByTestId("background-preview").style.filter).toBe(
      "blur(12px) saturate(150%) brightness(70%)",
    );
    await user.click(show);
    expect(currentDoc().settings.background).toBeNull();
  });

  it("uploads a background image and lists it for reuse", async () => {
    const { api } = await renderForm(<SettingsForm onClose={() => {}} />);
    const user = userEvent.setup();
    await user.click(screen.getByLabelText("Show a background image"));
    const file = new File(["x"], "wall.jpg", { type: "image/jpeg" });
    vi.spyOn(api, "uploadImage").mockResolvedValue({ name: "wall.jpg", url: "/images/wall.jpg" });
    await user.upload(screen.getByLabelText("Upload background image file"), file);
    expect(api.uploadImage).toHaveBeenCalledWith(file);
    expect(currentDoc().settings.background?.url).toBe("/images/wall.jpg");
    expect(await screen.findByRole("button", { name: "Use image wall.jpg" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("saves a copy of a web image on the server", async () => {
    const { api } = await renderForm(<SettingsForm onClose={() => {}} />);
    const user = userEvent.setup();
    await user.click(screen.getByLabelText("Show a background image"));
    expect(
      screen.queryByRole("button", { name: "Save a copy on this server" }),
    ).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("Image URL"), "https://cdn.example/a/wall.jpg");
    vi.spyOn(api, "fetchImage").mockResolvedValue({ name: "wall.jpg", url: "/images/wall.jpg" });
    await user.click(screen.getByRole("button", { name: "Save a copy on this server" }));
    expect(api.fetchImage).toHaveBeenCalledWith("https://cdn.example/a/wall.jpg");
    expect(currentDoc().settings.background?.url).toBe("/images/wall.jpg");
  });

  it("sets the frosted card blur", async () => {
    await renderForm(<SettingsForm onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Frosted glass blur (px)"), {
      target: { value: "8" },
    });
    expect(currentDoc().settings.cardBlur).toBe(8);
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
