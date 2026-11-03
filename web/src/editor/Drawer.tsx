/**
 * Side drawer that hosts the form or panel chosen with `open(target)`.
 *
 * Forms edit the working copy directly (live preview on the dashboard behind
 * the drawer). Each form session uses one coalesce key, so the whole session
 * is a single undo step and Cancel simply undoes it.
 */
import { useApp } from "../app/context";
import { BookmarkForm, BookmarkGroupForm } from "./forms/BookmarkForms";
import { GroupForm } from "./forms/GroupForm";
import { InfoWidgetsForm } from "./forms/InfoWidgetsForm";
import { ServiceForm } from "./forms/ServiceForm";
import { SettingsForm } from "./forms/SettingsForm";
import { HistoryPanel } from "./HistoryPanel";

/** Renders the drawer when a target is open. */
export function Drawer() {
  const { target, open } = useApp();
  if (!target) return null;
  const close = () => open(null);
  let body;
  switch (target.kind) {
    case "service":
      body = <ServiceForm groupId={target.groupId} serviceId={target.serviceId} onClose={close} />;
      break;
    case "group":
      body = <GroupForm groupId={target.groupId} onClose={close} />;
      break;
    case "bookmarkGroup":
      body = <BookmarkGroupForm groupId={target.groupId} onClose={close} />;
      break;
    case "bookmark":
      body = <BookmarkForm groupId={target.groupId} bookmarkId={target.bookmarkId} onClose={close} />;
      break;
    case "infoWidgets":
      body = <InfoWidgetsForm onClose={close} />;
      break;
    case "settings":
      body = <SettingsForm onClose={close} />;
      break;
    case "history":
      body = <HistoryPanel onClose={close} />;
      break;
  }
  return (
    <aside className="drawer" role="dialog" aria-modal="false" aria-label="Editor">
      {/* key forces a fresh form session (and coalesce key) per target */}
      <div key={JSON.stringify(target)} className="drawer-body">
        {body}
      </div>
    </aside>
  );
}
