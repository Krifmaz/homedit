import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BookmarkForm, BookmarkGroupForm, validateBookmark } from "./BookmarkForms";
import { currentDoc, renderForm } from "./testUtils";

afterEach(() => vi.restoreAllMocks());

const bookmark = (id: string) =>
  currentDoc()
    .bookmarks.flatMap((g) => g.items)
    .find((b) => b.id === id);

describe("BookmarkGroupForm", () => {
  it("renames a group live", async () => {
    await renderForm(<BookmarkGroupForm groupId="b-dev" onClose={() => {}} />);
    await userEvent.setup().type(screen.getByLabelText("Name"), "s");
    expect(currentDoc().bookmarks[0]!.name).toBe("Developers");
  });

  it("adds a new group", async () => {
    await renderForm(<BookmarkGroupForm onClose={() => {}} />);
    await userEvent.setup().type(screen.getByLabelText("Name"), "News");
    expect(currentDoc().bookmarks.at(-1)).toMatchObject({ name: "News", items: [] });
    expect(currentDoc().bookmarks).toHaveLength(3);
  });

  it("deletes after confirmation", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await renderForm(<BookmarkGroupForm groupId="b-social" onClose={() => {}} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Delete" }));
    expect(currentDoc().bookmarks.map((g) => g.id)).toEqual(["b-dev"]);
  });
});

describe("BookmarkForm", () => {
  it("edits fields live and limits the abbreviation to 2 characters", async () => {
    await renderForm(<BookmarkForm groupId="b-dev" bookmarkId="b-gh" onClose={() => {}} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Name"), "!");
    const abbr = screen.getByLabelText("Abbreviation");
    await user.clear(abbr);
    await user.type(abbr, "XYZ");
    expect(bookmark("b-gh")).toMatchObject({ name: "GitHub!", abbr: "XY" });
  });

  it("requires an http(s) link", async () => {
    await renderForm(<BookmarkForm groupId="b-dev" bookmarkId="b-gh" onClose={() => {}} />);
    const user = userEvent.setup();
    const href = screen.getByLabelText("Link");
    await user.clear(href);
    expect(screen.getByText("Link is required")).toBeInTheDocument();
    await user.type(href, "github.com");
    expect(screen.getByText("Link must start with http:// or https://")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
  });

  it("adds a new bookmark and can move it to another group", async () => {
    await renderForm(<BookmarkForm groupId="b-social" onClose={() => {}} />);
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Name"), "Lobsters");
    await user.type(screen.getByLabelText("Link"), "https://lobste.rs");
    expect(currentDoc().bookmarks[1]!.items.at(-1)).toMatchObject({
      name: "Lobsters",
      href: "https://lobste.rs",
    });
    await user.selectOptions(screen.getByLabelText("Bookmark group"), "b-dev");
    expect(currentDoc().bookmarks[0]!.items.at(-1)!.name).toBe("Lobsters");
    expect(currentDoc().bookmarks[1]!.items).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Done" })).toBeEnabled();
  });

  it("deletes after confirmation", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await renderForm(<BookmarkForm groupId="b-dev" bookmarkId="b-docs" onClose={() => {}} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Delete" }));
    expect(bookmark("b-docs")).toBeUndefined();
  });
});

describe("validateBookmark", () => {
  it("checks name, link and abbreviation", () => {
    expect(validateBookmark({ id: "x", name: "A", href: "https://a" })).toEqual({});
    expect(
      Object.keys(validateBookmark({ id: "x", name: "", href: "", abbr: "ABC" })).sort(),
    ).toEqual(["abbr", "href", "name"]);
  });
});
