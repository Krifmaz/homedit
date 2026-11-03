/**
 * Small helpers shared by the editor forms.
 */
import type { Group } from "../../api/types";

/** Whether `value` is an absolute `http://` or `https://` URL. */
export function isHttpUrl(value: string): boolean {
  return /^https?:\/\/\S+$/i.test(value.trim());
}

/** Returns `undefined` for an empty (or whitespace-only) string, else the string. */
export function optional(value: string): string | undefined {
  return value.trim() === "" ? undefined : value;
}

/** A group option for a `<select>`: its id and an indented label. */
export interface GroupOption {
  /** Group id. */
  id: string;
  /** Label showing the nesting, e.g. `Media / Downloads`. */
  label: string;
}

/**
 * Lists every group (nested ones included, depth first) with a label that
 * shows its path, for group pickers.
 *
 * @param groups top-level groups of the document.
 * @param exclude id of a group to leave out together with its sub-groups.
 */
export function groupOptions(groups: Group[], exclude?: string): GroupOption[] {
  const out: GroupOption[] = [];
  const walk = (list: Group[], prefix: string) => {
    for (const g of list) {
      if (g.id === exclude) continue;
      const label = prefix ? `${prefix} / ${g.name || "(unnamed)"}` : g.name || "(unnamed)";
      out.push({ id: g.id, label });
      if (g.groups) walk(g.groups, label);
    }
  };
  walk(groups, "");
  return out;
}

/** Counts the cards of a group, its sub-groups' cards included. */
export function countCards(group: Group): number {
  return group.items.length + (group.groups ?? []).reduce((n, g) => n + countCards(g), 0);
}
