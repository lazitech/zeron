# Mouse side button navigation

The user approved using the mouse Back and Forward buttons for the navigation
history under the pointer. Over Zeron's embedded browser surface, the buttons
move through that page's web history. Elsewhere in the app, they use Zeron's
existing route history, matching the titlebar Back and Forward controls.

The shell handles `MouseButton::Navigate(Back)` and
`MouseButton::Navigate(Forward)` as the app-level fallback. The embedded
browser consumes those events before they reach the shell and calls its
existing page-history operation. Linux's custom WebKit surface can forward the
GPUI navigation buttons directly to that operation. On macOS, the native
WKWebView is a child view outside GPUI's normal mouse dispatch, so its native
event path must detect side-button presses over the visible browser surface,
move its web history, and consume the event to avoid duplicate navigation.
Other app locations let the event continue to the shell route handler.

No new navigation stack, keyboard shortcut, preference, or toolbar control is
needed. Existing history boundaries remain authoritative: pressing Back at the
oldest entry or Forward at the newest entry is a no-op. On platforms where
Zeron opens websites in the external browser, the side buttons continue to
navigate Zeron's own route history.

Validation targets: Back and Forward change browser history over an embedded
browser surface; the same buttons change Zeron route history elsewhere; a
press at either history boundary is harmless; and macOS browser presses are
handled once by the native WebView path rather than also changing the shell
route.
