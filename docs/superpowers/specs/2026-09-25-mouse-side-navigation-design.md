# Mouse side button navigation

The user approved using the mouse Back and Forward buttons for the navigation
history under the pointer. Over Zeron's embedded browser surface, the buttons
move through that page's web history. Elsewhere in the app, they use Zeron's
existing route history, matching the titlebar Back and Forward controls.

The shell handles `MouseButton::Navigate(Back)` and
`MouseButton::Navigate(Forward)` as the app-level fallback. The embedded
browser consumes those GPUI events before they reach the shell and calls its
existing page-history operation. Linux's custom WebKit surface receives GPUI
events directly. On macOS, the locked Wry WebView already maps native
other-mouse down/up events for buttons 3 and 4 to browser Back/Forward, so the
app relies on that native path and adds no second listener. Other app
locations let GPUI deliver the event to the shell route handler.

No new navigation stack, keyboard shortcut, preference, or toolbar control is
needed. Existing history boundaries remain authoritative: pressing Back at the
oldest entry or Forward at the newest entry is a no-op. On platforms where
Zeron opens websites in the external browser, the side buttons continue to
navigate Zeron's own route history.

Validation targets: Back and Forward change browser history over an embedded
browser surface; the same buttons change Zeron route history elsewhere; a
press at either history boundary is harmless; and macOS browser presses are
handled once by Wry's native WebView path rather than also changing the shell
route.
