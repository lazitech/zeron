//! Context occupancy is read from the replicated chat snapshot, never local CLI state.
use crate::theme::Theme;
use gpui::{
    Context, IntoElement, PathBuilder, Render, SharedString, Window, canvas, div, point,
    prelude::*, px,
};
use zeron_proto::{ContextUsage, SessionUsage};

pub fn render(
    usage: Option<ContextUsage>,
    state: gpui::Entity<crate::state::AppState>,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let fraction = usage.and_then(ContextUsage::fraction);
    let color = match fraction {
        Some(f) if f >= 0.9 => theme.danger,
        Some(f) if f >= 0.75 => theme.warning,
        Some(_) => theme.text_muted,
        None => theme.text_faint,
    };
    let track = theme.text_faint.opacity(0.25);
    let ring = canvas(
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
            arc(fraction.unwrap_or(0.0).clamp(0.0, 1.0) as f32, color);
        },
    )
    .size(px(16.0));
    let label = fraction
        .map(|f| format!("{:.0}%", f * 100.0))
        .unwrap_or_else(|| "—".into());
    div()
        .id("context-usage")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(5.0))
        .h(px(24.0))
        .px(px(6.0))
        .rounded(px(6.0))
        .text_size(px(11.0))
        .text_color(color)
        .hover(|s| s.bg(crate::theme::ink(0.05)))
        .child(ring)
        .child(SharedString::from(label))
        .tooltip(move |_, cx| {
            cx.new(|cx| UsageCard {
                _subscription: cx.observe(&state, |_, _, cx| cx.notify()),
                state: state.clone(),
            })
            .into()
        })
}

struct UsageCard {
    state: gpui::Entity<crate::state::AppState>,
    _subscription: gpui::Subscription,
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
        assert_eq!(token_value(Some(1_048_576)), "1,048,576");
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
