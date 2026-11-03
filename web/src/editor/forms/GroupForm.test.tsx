import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { findGroup } from "../ops";
import { GroupForm } from "./GroupForm";
import { currentDoc, renderForm } from "./testUtils";

afterEach(() => vi.restoreAllMocks());

describe("GroupForm", () => {
  it("edits name, visibility and layout live", async () => {
    await renderForm(<GroupForm groupId="g-media" onClose={() => {}} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Name"), " & TV");
    await user.click(screen.getByLabelText("Hidden from viewers"));
    await user.selectOptions(screen.getByLabelText("Layout"), "row");
    const cols = screen.getByLabelText("Columns");
    await user.tripleClick(cols);
    await user.keyboard("5");
    const g = findGroup(currentDoc(), "g-media")!;
    expect(g.name).toBe("Media & TV");
    expect(g.hidden).toBe(true);
    expect(g.layout).toEqual({ columns: 5, style: "row" });
    expect(g.items).toHaveLength(4);
    expect(screen.queryByLabelText("Parent group")).not.toBeInTheDocument();
  });

  it("cancel reverts the changes", async () => {
    await renderForm(<GroupForm groupId="g-media" onClose={() => {}} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Name"), "!!");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(findGroup(currentDoc(), "g-media")!.name).toBe("Media");
  });

  it("adds a new group at the top level by default", async () => {
    await renderForm(<GroupForm onClose={() => {}} />);
    const user = userEvent.setup();
    expect(screen.getByLabelText("Parent group")).toHaveValue("");
    await user.type(screen.getByLabelText("Name"), "Home");
    const last = currentDoc().groups.at(-1)!;
    expect(last.name).toBe("Home");
    expect(last.items).toEqual([]);
  });

  it("adds a new group under a chosen parent", async () => {
    await renderForm(<GroupForm onClose={() => {}} />);
    const user = userEvent.setup();
    await user.selectOptions(screen.getByLabelText("Parent group"), "g-infra");
    await user.type(screen.getByLabelText("Name"), "Net");
    expect(findGroup(currentDoc(), "g-infra")!.groups?.map((g) => g.name)).toEqual(["Net"]);
    expect(currentDoc().groups.some((g) => g.name === "Net")).toBe(false);
    // Re-parenting after insertion moves it.
    await user.selectOptions(screen.getByLabelText("Parent group"), "");
    expect(findGroup(currentDoc(), "g-infra")!.groups ?? []).toHaveLength(0);
    expect(currentDoc().groups.at(-1)!.name).toBe("Net");
  });

  it("confirms deletion with the card count", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const onClose = vi.fn();
    await renderForm(<GroupForm groupId="g-infra" onClose={onClose} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Delete" }));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("2 cards"));
    expect(findGroup(currentDoc(), "g-infra")).toBeUndefined();
    expect(onClose).toHaveBeenCalled();
  });
});
