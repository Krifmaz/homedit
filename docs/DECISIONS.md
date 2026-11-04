# Decisions

Non-obvious calls made while building homedit, newest first.

## 2026-11-04: Offer a /24 scan for bare network addresses at start and on empty results

Entering `10.0.1.0` scans that one (usually unused) address and finds nothing.
A hint under the targets box already suggested `/24` since v0.1.3, but it was
easy to miss. Now Start scan asks "Scan 10.0.1.0/24 instead?" (yes widens the
box and scans the network, no scans the single address as typed), and a
finished scan of a bare address that found nothing offers a one-click
"Scan 10.0.1.0/24".

- The server still treats `10.0.1.0` as a single address. Rewriting it
  server-side would silently change what an admin asked for (and break
  saved rescan schedules that target exactly that address), so the
  widening stays an explicit choice in the UI.
- The question uses `window.confirm` like the existing "all 65,535 ports"
  prompt. When the kit's confirm dialog is adopted, both switch together.
