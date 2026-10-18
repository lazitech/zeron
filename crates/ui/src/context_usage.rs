//! Context occupancy is read from the replicated chat snapshot, never local CLI state.
use crate::theme::Theme;
use gpui::{
    Context, Entity, IntoElement, PathBuilder, Render, SharedString, Window, canvas, div, point,
    prelude::*, px,
};
use zeron_proto::{ContextUsage, SessionUsage};

/// The context ring's trigger chip; the footer ([`crate::account_usage`])
/// opens [`card`] from it on click.
pub fn chip(usage: Option<ContextUsage>, open: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let fraction = usage.and_then(ContextUsage::fraction);
    let color = match fraction {
        Some(f) if f >= 0.9 => theme.danger,
        Some(f) if f >= 0.75 => theme.warning,
        Some(_) => theme.text_muted,
        None => theme.text_faint,
    };
    let label = fraction
        .map(|f| format!("{:.0}%", f * 100.0))
        .unwrap_or_else(|| "—".into());
    ring_chip(
        "context-usage",
        fraction.unwrap_or(0.0) as f32,
        color,
        color,
        label,
        open,
        theme,
    )
}

/// One footer ring indicator: ring + percent, identical geometry for every
/// ring so they sit side by side as equals. `arc` colours the ring's fill,
/// `text` the label; `open` holds the hover wash while its popover is up.
pub(crate) fn ring_chip(
    id: &'static str,
    fraction: f32,
    arc: gpui::Hsla,
    text: gpui::Hsla,
    label: String,
    open: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(5.0))
        .h(px(24.0))
        .px(px(6.0))
        .rounded(px(6.0))
        .text_size(px(11.0))
        .text_color(text)
        .cursor_pointer()
        .when(open, |s| s.bg(crate::theme::ink(0.05)))
        .hover(|s| s.bg(crate::theme::ink(0.05)))
        .child(ring(fraction, arc, theme))
        .child(SharedString::from(label))
}

/// The footer's 16px progress ring: a faint full track under a `fraction` arc
/// starting at twelve o'clock. Shared with the account usage indicator.
pub(crate) fn ring(fraction: f32, color: gpui::Hsla, theme: &Theme) -> impl IntoElement {
    let track = theme.text_faint.opacity(0.25);
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let mut arc = |fraction: f32, color| {
                if fraction <= 0.0 {
                    return;
                }
                let steps = (64.0 * fraction).ceil().max(2.0) as usize;
                let mut path = PathBuilder::stroke(px(1.8));
                for i in 0..=steps {
                    let angle = -std::f32::consts::FRAC_PI_2
                        + std::f32::consts::TAU * fraction * i as f32 / steps as f32;
                    let p = point(
                        center.x + px(6.0 * angle.cos()),
                        center.y + px(6.0 * angle.sin()),
                    );
                    if i == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            };
            arc(1.0, track);
            arc(fraction.clamp(0.0, 1.0), color);
        },
    )
    .size(px(16.0))
}

fn with_separators(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Whether the context ring has a reported window to measure against.
pub fn has_window(usage: Option<ContextUsage>) -> bool {
    usage
        .and_then(|usage| usage.window)
        .is_some_and(|window| window > 0)
}

/// Whether the footer should show the indicator and its session usage card.
/// Session usage can be available even when a harness does not report a
/// context window, in which case the indicator keeps its dash label.
pub fn has_any_usage(
    context_usage: Option<ContextUsage>,
    session_usage: Option<SessionUsage>,
) -> bool {
    has_window(context_usage) || session_usage.is_some()
}

fn details(usage: Option<ContextUsage>) -> String {
    match usage.unwrap_or_default() {
        ContextUsage {
            tokens: Some(tokens),
            window: Some(window),
        } if window > 0 => {
            format!(
                "{} / {} tokens\n{} tokens remaining",
                with_separators(tokens),
                with_separators(window),
                with_separators(window.saturating_sub(tokens))
            )
        }
        ContextUsage {
            tokens: Some(tokens),
            ..
        } => format!(
            "{} tokens used\nContext limit not reported",
            with_separators(tokens)
        ),
        ContextUsage {
            window: Some(window),
            ..
        } if window > 0 => format!(
            "{} token capacity\nWaiting for context usage",
            with_separators(window)
        ),
        _ => "Context usage not reported by this harness yet".into(),
    }
}

fn token_value(value: Option<u64>) -> String {
    value.map(with_separators).unwrap_or_else(|| "—".into())
}

fn cache_hit_rate_value(usage: Option<SessionUsage>) -> String {
    usage
        .and_then(|usage| usage.cache_hit_rate())
        .map(|rate| format!("{:.1}%", rate * 100.0))
        .unwrap_or_else(|| "—".into())
}

fn metric_row(label: &'static str, value: String, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.0))
        .child(
            div()
                .text_color(theme.text_muted)
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .text_right()
                .text_color(theme.text)
                .child(SharedString::from(value)),
        )
}

/// The local session token usage card shown when hovering the footer rings.
pub struct UsageCard {
    state: Entity<crate::state::AppState>,
    _subscription: gpui::Subscription,
}

/// Add the session token tooltip to a footer element without changing its
/// visual children. This lets the upstream account quota and context rings
/// remain a single cluster while retaining the local token details.
///
/// Only harnesses that report cumulative session totals (Codex, OpenCode,
/// Pi) have anything to show; without a snapshot the card would be three
/// dashes over the account ring, so no tooltip is attached.
pub fn with_session_tooltip(
    element: gpui::Stateful<gpui::Div>,
    state: Entity<crate::state::AppState>,
    cx: &gpui::App,
) -> gpui::Stateful<gpui::Div> {
    if state.read(cx).session_usage.is_none() {
        return element;
    }
    element.tooltip(move |_, cx| {
        cx.new(|cx| UsageCard {
            _subscription: cx.observe(&state, |_, _, cx| cx.notify()),
            state: state.clone(),
        })
        .into()
    })
}

impl Render for UsageCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &Theme::of(cx).for_popup();
        let session_usage = self.state.read(cx).session_usage;
        let card = crate::popover::popover_card(theme)
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .text_size(px(12.0))
            .line_height(px(19.0))
            .whitespace_nowrap()
            .children([
                metric_row(
                    "Input tokens",
                    token_value(session_usage.and_then(|usage| usage.input_tokens)),
                    theme,
                ),
                metric_row(
                    "Output tokens",
                    token_value(session_usage.and_then(|usage| usage.output_tokens)),
                    theme,
                ),
                metric_row("Cache hit rate", cache_hit_rate_value(session_usage), theme),
            ]);
        crate::frost::frosted(crate::popover::CARD_RADIUS, crate::frost::MENU_BLUR, card)
    }
}

/// The context ring's popover content.
pub fn card(usage: Option<ContextUsage>, theme: &Theme) -> gpui::Div {
    crate::popover::popover_card(theme)
        .flex()
        .flex_col()
        .child(crate::popover::menu_heading(theme, "Context window"))
        .child(
            div()
                .px(px(8.0))
                .pb(px(6.0))
                .text_size(px(12.0))
                .line_height(px(19.0))
                .whitespace_nowrap()
                .text_color(theme.text_muted)
                .child(SharedString::from(details(usage))),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indicator_needs_a_reported_window() {
        assert!(!has_window(None));
        assert!(!has_window(Some(ContextUsage {
            tokens: Some(1_200),
            window: None,
        })));
        assert!(!has_window(Some(ContextUsage {
            tokens: Some(1_200),
            window: Some(0),
        })));
        assert!(has_window(Some(ContextUsage {
            tokens: None,
            window: Some(200_000),
        })));
    }

    #[test]
    fn missing_usage_is_distinct_from_zero_and_overflow() {
        assert!(details(None).contains("not reported"));
        assert!(
            details(Some(ContextUsage {
                tokens: Some(0),
                window: Some(200)
            }))
            .contains("200 tokens remaining")
        );
        assert!(
            details(Some(ContextUsage {
                tokens: Some(250),
                window: Some(200)
            }))
            .contains("0 tokens remaining")
        );
        assert!(
            details(Some(ContextUsage {
                tokens: Some(10),
                window: Some(0)
            }))
            .contains("limit not reported")
        );
    }

    #[test]
    fn session_usage_without_context_still_shows_indicator() {
        assert!(!has_any_usage(None, None));
        assert!(has_any_usage(
            Some(ContextUsage {
                tokens: None,
                window: Some(200_000),
            }),
            None,
        ));
        assert!(has_any_usage(None, Some(SessionUsage::default())));
    }

    #[test]
    fn token_counts_are_grouped_by_thousands() {
        assert_eq!(with_separators(0), "0");
        assert_eq!(with_separators(999), "999");
        assert_eq!(with_separators(5417), "5,417");
        assert_eq!(with_separators(1_048_576), "1,048,576");
        assert_eq!(
            details(Some(ContextUsage {
                tokens: Some(5417),
                window: Some(1_048_576)
            })),
            "5,417 / 1,048,576 tokens\n1,043,159 tokens remaining"
        );
    }

    #[test]
    fn session_usage_values_show_dashes_until_reported() {
        assert_eq!(token_value(None), "—");
        assert_eq!(token_value(Some(0)), "0");
        assert_eq!(token_value(Some(1_048_576)), "1,048,576");
        assert_eq!(cache_hit_rate_value(None), "—");
        assert_eq!(
            cache_hit_rate_value(Some(SessionUsage {
                input_tokens: Some(0),
                output_tokens: Some(4),
                cached_input_tokens: Some(0),
            })),
            "—"
        );
    }

    #[test]
    fn session_usage_cache_hit_rate_is_formatted_as_percent() {
        assert_eq!(
            cache_hit_rate_value(Some(SessionUsage {
                input_tokens: Some(1_000),
                output_tokens: Some(50),
                cached_input_tokens: Some(800),
            })),
            "80.0%"
        );
        assert_eq!(
            cache_hit_rate_value(Some(SessionUsage {
                input_tokens: Some(1_000),
                output_tokens: Some(50),
                cached_input_tokens: Some(1_001),
            })),
            "—"
        );
    }
}
