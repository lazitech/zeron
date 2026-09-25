//! Local usage statistics dashboard.

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Weekday};
use gpui::{
    AnyElement, Context, Entity, Hsla, SharedString, Subscription, Window, div, prelude::*, px,
};

use crate::popover::{self, Loadable, ScrollRailHost};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;
use zeron_proto::{
    UsageBucket, UsageDay, UsageDistribution, UsageRank, UsageStatistics, UsageTrendWeek,
};

const TREND_OTHER_ID: &str = "__other__";
const HEATMAP_CELL_SIZE: f32 = 20.0;
const HEATMAP_CELL_GAP: f32 = 4.0;
const HEATMAP_LEGEND_CELL_SIZE: f32 = 12.0;
const HEATMAP_MONTH_LABEL_HEIGHT: f32 = 16.0;
const HEATMAP_COLUMN_STEP: f32 = HEATMAP_CELL_SIZE + HEATMAP_CELL_GAP;
const HEATMAP_WEEKDAY_LABEL_WIDTH: f32 = 16.0;
const HEATMAP_LEFT_GUTTER: f32 = 24.0;
const HEATMAP_WEEKDAY_GAP: f32 = HEATMAP_LEFT_GUTTER - HEATMAP_WEEKDAY_LABEL_WIDTH;
const TREND_LEFT_SPACER: f32 = HEATMAP_LEFT_GUTTER - HEATMAP_CELL_GAP;
const TREND_CHART_HEIGHT: f32 = 136.0;
const TREND_BAR_MAX_HEIGHT: f32 = 128.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RankingMetric {
    Tokens,
    Interactions,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DistributionKind {
    Hours,
    Weekdays,
    Months,
}

impl DistributionKind {
    fn label(self) -> &'static str {
        match self {
            Self::Hours => "小时",
            Self::Weekdays => "星期",
            Self::Months => "月份",
        }
    }
}

impl RankingMetric {
    fn value(self, rank: &UsageRank) -> Option<u64> {
        match self {
            Self::Tokens => rank.has_token_data.then_some(rank.tokens),
            Self::Interactions => Some(rank.prompts),
        }
    }

    fn format(self, value: u64) -> String {
        match self {
            Self::Tokens => format_tokens(value),
            Self::Interactions => format_count(value),
        }
    }
}

pub struct StatisticsPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    snapshot: Loadable<Arc<UsageStatistics>>,
    ranking_metrics: [RankingMetric; 3],
    distribution_kind: DistributionKind,
    _observe: Subscription,
}

impl StatisticsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |this, state, cx| {
            let state = state.read(cx);
            if let Some(snapshot) = state.usage_statistics.clone() {
                let unchanged = this
                    .snapshot
                    .ready()
                    .is_some_and(|current| Arc::ptr_eq(current, &snapshot));
                if !unchanged {
                    this.snapshot = Loadable::Ready(snapshot);
                }
            } else if state.usage_statistics_loading {
                this.snapshot = Loadable::Loading;
            } else if let Some(error) = state.usage_statistics_error.clone() {
                this.snapshot = Loadable::Error(error);
            } else {
                this.snapshot = Loadable::Idle;
            }
            cx.notify();
        });
        let snapshot = {
            let state = state.read(cx);
            if let Some(snapshot) = state.usage_statistics.clone() {
                Loadable::Ready(snapshot)
            } else if state.usage_statistics_loading {
                Loadable::Loading
            } else if let Some(error) = state.usage_statistics_error.clone() {
                Loadable::Error(error)
            } else {
                Loadable::Idle
            }
        };
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            snapshot,
            ranking_metrics: [RankingMetric::Tokens; 3],
            distribution_kind: DistributionKind::Hours,
            _observe: observe,
        };
        page.refresh_if_needed(cx);
        page
    }

    fn refresh_if_needed(&mut self, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.request_usage_statistics(cx));
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn page_heading(theme: &Theme) -> AnyElement {
        div()
            .w_full()
            .child(widgets::page_header(theme, "统计", None))
            .into_any_element()
    }

    fn overview(&self, stats: &UsageStatistics, theme: &Theme) -> AnyElement {
        let totals = &stats.totals;
        let metrics = [
            (
                "Token 总计",
                if totals.has_token_data {
                    format_tokens(totals.tokens)
                } else {
                    "—".into()
                },
                true,
            ),
            (
                "今天 Token",
                totals
                    .today_tokens
                    .map(format_tokens)
                    .unwrap_or_else(|| "—".into()),
                true,
            ),
            ("交互轮数", format_count(totals.prompts), false),
            ("活跃天数", format_count(totals.active_days), false),
        ];
        div()
            .w_full()
            .flex()
            .flex_row()
            .pb(px(16.0))
            .border_b_1()
            .border_color(theme.border)
            .children(
                metrics
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, value, tokens))| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(index > 0, |cell| {
                                cell.pl(px(12.0)).border_l_1().border_color(theme.border)
                            })
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(if tokens {
                                        20.0
                                    } else {
                                        17.0
                                    }))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if tokens { theme.accent } else { theme.text })
                                    .truncate()
                                    .child(SharedString::from(value)),
                            )
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(11.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(label)),
                            )
                    }),
            )
            .into_any_element()
    }

    fn recent_comparison(&self, stats: &UsageStatistics, theme: &Theme) -> AnyElement {
        let current_interactions = interaction_total(&stats.recent_days);
        let previous_interactions = interaction_total(&stats.previous_days);
        let current_active_days = active_days(&stats.recent_days);
        let previous_active_days = active_days(&stats.previous_days);
        let rows = [
            ("交互轮数", current_interactions, previous_interactions),
            ("活跃天数", current_active_days, previous_active_days),
        ];
        let cells = rows.into_iter().map(|(label, now, before)| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(label)),
                )
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(15.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(SharedString::from(format_count(now))),
                )
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(change_color(now, before, theme))
                        .child(SharedString::from(format!(
                            "前 7 天 {} · {}",
                            format_count(before),
                            percent_change(now, before)
                        ))),
                )
                .into_any_element()
        });
        let week_token_label = if stats.totals.week_tokens.is_some() {
            "周一至今"
        } else {
            "暂无可按日期统计的用量"
        };
        let week_token_cell = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .child(
                div()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from("本周 Token")),
            )
            .child(
                div()
                    .text_size(crate::typography::ui_rems(15.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(if stats.totals.week_tokens.is_some() {
                        theme.accent
                    } else {
                        theme.text_faint
                    })
                    .child(SharedString::from(
                        stats
                            .totals
                            .week_tokens
                            .map(format_tokens)
                            .unwrap_or_else(|| "—".into()),
                    )),
            )
            .child(
                div()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(week_token_label)),
            )
            .into_any_element();
        let cells = cells.chain(std::iter::once(week_token_cell));

        statistics_section(theme)
            .gap(px(14.0))
            .child(section_heading(
                theme,
                "最近 7 天",
                &format!(
                    "{} · 对比 {}",
                    date_range(&stats.recent_days),
                    date_range(&stats.previous_days)
                ),
            ))
            .child(div().flex().flex_row().gap(px(24.0)).children(cells))
            .into_any_element()
    }

    fn activity_heatmap(&self, stats: &UsageStatistics, theme: &Theme) -> AnyElement {
        let max_activity = stats
            .heatmap
            .iter()
            .map(|day| day.prompts)
            .max()
            .unwrap_or(0);
        let leading = stats
            .heatmap
            .first()
            .and_then(|day| NaiveDate::parse_from_str(&day.day, "%Y-%m-%d").ok())
            .map_or(0, |date| date.weekday().num_days_from_monday() as usize);
        let mut aligned: Vec<Option<&UsageDay>> = vec![None; leading];
        aligned.extend(stats.heatmap.iter().map(Some));
        while aligned.len() % 7 != 0 {
            aligned.push(None);
        }
        let weeks: Vec<Vec<Option<&UsageDay>>> =
            aligned.chunks_exact(7).map(|week| week.to_vec()).collect();
        let month_labels = heatmap_month_labels(&weeks);
        let month_labels = month_labels.iter().map(|(index, label)| {
            div()
                .absolute()
                .left(px(*index as f32 * HEATMAP_COLUMN_STEP))
                .top(px(0.0))
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_faint)
                .child(SharedString::from(label.clone()))
        });
        let weeks = weeks.iter().map(|days| {
            div()
                .flex()
                .flex_col()
                .gap(px(HEATMAP_CELL_GAP))
                .flex_none()
                .children(days.iter().map(|day| match day {
                    Some(day) => heat_cell(day, max_activity, theme).into_any_element(),
                    None => div().size(px(HEATMAP_CELL_SIZE)).into_any_element(),
                }))
        });
        let weekday_labels = ["一", "", "三", "", "五", "", ""];
        let mut activity_notes = Vec::new();
        if stats.totals.current_streak_days > 0 {
            activity_notes.push(format!("当前连续 {} 天", stats.totals.current_streak_days));
        }
        if stats.totals.longest_streak_days > 0 {
            activity_notes.push(format!("最长连续 {} 天", stats.totals.longest_streak_days));
        }
        if let Some(busiest_day) = stats.totals.busiest_day.as_deref() {
            activity_notes.push(format!("最忙的一天 {}", format_day_label(busiest_day)));
        }
        statistics_section(theme)
            .gap(px(14.0))
            .child(section_heading(theme, "活动热力图", ""))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(HEATMAP_WEEKDAY_GAP))
                    .child(
                        div()
                            .mt(px(HEATMAP_MONTH_LABEL_HEIGHT + HEATMAP_CELL_GAP))
                            .flex()
                            .flex_col()
                            .gap(px(HEATMAP_CELL_GAP))
                            .children(weekday_labels.into_iter().map(|label| {
                                div()
                                    .w(px(HEATMAP_WEEKDAY_LABEL_WIDTH))
                                    .h(px(HEATMAP_CELL_SIZE))
                                    .text_size(crate::typography::ui_rems(10.0))
                                    .text_color(theme.text_faint)
                                    .child(SharedString::from(label))
                            })),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(HEATMAP_CELL_GAP))
                            .child(
                                div()
                                    .relative()
                                    .w_full()
                                    .h(px(HEATMAP_MONTH_LABEL_HEIGHT))
                                    .children(month_labels),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(HEATMAP_CELL_GAP))
                                    .children(weeks),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(18.0))
                    .children(
                        activity_notes
                            .into_iter()
                            .map(|note| quiet_text(theme, note)),
                    )
                    .child(heatmap_legend(theme)),
            )
            .into_any_element()
    }

    fn agent_trend(&self, stats: &UsageStatistics, theme: &Theme) -> AnyElement {
        let agents = trend_agents(&stats.agent_trend);
        let top_agent_ids = agents
            .iter()
            .filter(|(agent_id, _)| agent_id.as_str() != TREND_OTHER_ID)
            .map(|(agent_id, _)| agent_id.clone())
            .collect::<BTreeSet<_>>();
        let max_total = stats
            .agent_trend
            .iter()
            .map(|week| week.agents.iter().map(|agent| agent.prompts).sum::<u64>())
            .max()
            .unwrap_or(0)
            .max(1);
        let current_leader = stats.agent_trend.last().and_then(|week| {
            week.agents
                .iter()
                .filter(|agent| agent.prompts > 0)
                .max_by_key(|agent| agent.prompts)
        });
        let trend_subtitle = current_leader
            .map(|agent| format!("本周最多：{}", agent.agent_label))
            .unwrap_or_default();
        let weeks = stats.agent_trend.iter().map(|week| {
            let total = week.agents.iter().map(|agent| agent.prompts).sum::<u64>();
            let mut details = week
                .agents
                .iter()
                .filter(|agent| agent.prompts > 0)
                .map(|agent| (agent.agent_label.as_str(), agent.prompts))
                .collect::<Vec<_>>();
            details.sort_by(|left, right| right.1.cmp(&left.1));
            let breakdown = details
                .into_iter()
                .take(3)
                .map(|(label, count)| format!("{label} {count}"))
                .collect::<Vec<_>>()
                .join(" · ");
            let tooltip = if breakdown.is_empty() {
                format!("周起始 {} · 暂无交互轮数", compact_date(&week.week_start))
            } else {
                format!(
                    "周起始 {} · {} 轮交互\n{}",
                    compact_date(&week.week_start),
                    format_count(total),
                    breakdown
                )
            };
            let segments = agents.iter().filter_map(|(agent_id, _)| {
                let prompts = if agent_id.as_str() == TREND_OTHER_ID {
                    week.agents
                        .iter()
                        .filter(|agent| !top_agent_ids.contains(&agent.agent_id))
                        .map(|agent| agent.prompts)
                        .sum()
                } else {
                    week.agents
                        .iter()
                        .find(|agent| agent.agent_id == *agent_id)
                        .map(|agent| agent.prompts)
                        .unwrap_or(0)
                };
                (prompts > 0).then(|| {
                    div()
                        .w(px(HEATMAP_CELL_SIZE))
                        .h(px((prompts as f32 / max_total as f32
                            * TREND_BAR_MAX_HEIGHT)
                            .max(3.0)))
                        .bg(agent_color(theme, agent_id))
                        .into_any_element()
                })
            });
            div()
                .id(SharedString::from(format!(
                    "stats-trend-{}",
                    week.week_start
                )))
                .h(px(TREND_CHART_HEIGHT))
                .w(px(HEATMAP_CELL_SIZE))
                .flex()
                .flex_col()
                .justify_end()
                .gap(px(1.0))
                .flex_none()
                .children(segments)
                .tooltip(move |_, cx| {
                    cx.new(|_| StatisticsTooltip {
                        text: tooltip.clone(),
                    })
                    .into()
                })
                .into_any_element()
        });
        let month_labels =
            trend_month_labels(&stats.agent_trend)
                .into_iter()
                .map(|(index, label)| {
                    div()
                        .absolute()
                        .left(px(index as f32 * HEATMAP_COLUMN_STEP))
                        .top_0()
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_faint)
                        .child(SharedString::from(label))
                });
        let legend = agents.iter().map(|(agent_id, label)| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .size(px(8.0))
                        .rounded(px(2.0))
                        .bg(agent_color(theme, agent_id)),
                )
                .child(quiet_text(theme, label.clone()))
        });
        statistics_section(theme)
            .gap(px(12.0))
            .child(section_heading(theme, "时间变化", &trend_subtitle))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_end()
                            .gap(px(HEATMAP_CELL_GAP))
                            .child(
                                div()
                                    .w(px(TREND_LEFT_SPACER))
                                    .h(px(TREND_CHART_HEIGHT))
                                    .flex_none(),
                            )
                            .children(weeks),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(HEATMAP_CELL_GAP))
                            .child(div().w(px(TREND_LEFT_SPACER)).flex_none())
                            .child(
                                div()
                                    .relative()
                                    .w_full()
                                    .h(px(HEATMAP_MONTH_LABEL_HEIGHT))
                                    .children(month_labels),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(12.0))
                    .children(legend),
            )
            .into_any_element()
    }

    fn distribution(
        &self,
        stats: &UsageStatistics,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kind = self.distribution_kind;
        let buckets = distribution_buckets(&stats.distribution, kind);
        let max = buckets
            .iter()
            .map(|bucket| bucket.value)
            .max()
            .unwrap_or(0)
            .max(1);
        let peak = buckets.iter().max_by_key(|bucket| bucket.value);
        let peak_index = buckets
            .iter()
            .enumerate()
            .max_by_key(|(_, bucket)| bucket.value)
            .map(|(index, _)| index);
        let bars = buckets.iter().enumerate().map(|(index, bucket)| {
            let height = if bucket.value == 0 {
                2.0
            } else {
                (bucket.value as f32 / max as f32 * 84.0).max(4.0)
            };
            let show_label = match kind {
                DistributionKind::Hours => index % 6 == 0,
                DistributionKind::Weekdays | DistributionKind::Months => true,
            };
            let bucket_label = match kind {
                DistributionKind::Hours => format!("{} 时", bucket.label),
                DistributionKind::Weekdays | DistributionKind::Months => bucket.label.clone(),
            };
            let tooltip = format!("{} · {} 轮交互", bucket_label, format_count(bucket.value));
            div()
                .id(SharedString::from(format!(
                    "stats-distribution-{}-{index}",
                    kind.label()
                )))
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_end()
                .gap(px(5.0))
                .child(
                    div()
                        .w_full()
                        .max_w(px(20.0))
                        .h(px(height))
                        .rounded(px(3.0))
                        .bg(if peak_index == Some(index) && bucket.value > 0 {
                            theme.accent
                        } else {
                            theme
                                .accent
                                .opacity(if bucket.value == 0 { 0.1 } else { 0.62 })
                        }),
                )
                .child(
                    div()
                        .h(px(12.0))
                        .text_size(crate::typography::ui_rems(9.0))
                        .text_color(theme.text_faint)
                        .child(SharedString::from(if show_label {
                            bucket.label.clone()
                        } else {
                            String::new()
                        })),
                )
                .tooltip(move |_, cx| {
                    let text = tooltip.clone();
                    cx.new(|_| StatisticsTooltip { text }).into()
                })
        });
        let peak_copy = peak
            .filter(|bucket| bucket.value > 0)
            .map(|bucket| {
                let label = match kind {
                    DistributionKind::Hours => format!("{} 时", bucket.label),
                    DistributionKind::Weekdays | DistributionKind::Months => bucket.label.clone(),
                };
                format!("峰值在 {label}，共 {} 轮交互", format_count(bucket.value))
            })
            .unwrap_or_else(|| "暂无交互轮数记录".into());
        let controls = div()
            .flex()
            .flex_row()
            .gap(px(4.0))
            .child(distribution_toggle(
                theme,
                DistributionKind::Hours.label(),
                kind == DistributionKind::Hours,
                "stats-distribution-hours",
                cx.listener(|this, _, _, cx| {
                    this.distribution_kind = DistributionKind::Hours;
                    cx.notify();
                }),
            ))
            .child(distribution_toggle(
                theme,
                DistributionKind::Weekdays.label(),
                kind == DistributionKind::Weekdays,
                "stats-distribution-weekdays",
                cx.listener(|this, _, _, cx| {
                    this.distribution_kind = DistributionKind::Weekdays;
                    cx.notify();
                }),
            ))
            .child(distribution_toggle(
                theme,
                DistributionKind::Months.label(),
                kind == DistributionKind::Months,
                "stats-distribution-months",
                cx.listener(|this, _, _, cx| {
                    this.distribution_kind = DistributionKind::Months;
                    cx.notify();
                }),
            ));
        statistics_section(theme)
            .gap(px(14.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(section_heading(theme, "按时间分布", &peak_copy))
                    .child(controls),
            )
            .child(
                div()
                    .h(px(116.0))
                    .flex()
                    .flex_row()
                    .items_end()
                    .gap(px(4.0))
                    .children(bars),
            )
            .into_any_element()
    }

    fn rankings(
        &self,
        stats: &UsageStatistics,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(24.0))
            .child(self.ranking_section("Agents", &stats.agents, "agents", 0, theme, cx))
            .when(!stats.projects.is_empty(), |page| {
                page.child(self.ranking_section(
                    "Projects",
                    &stats.projects,
                    "projects",
                    1,
                    theme,
                    cx,
                ))
            })
            .when(!stats.models.is_empty(), |page| {
                page.child(self.ranking_section("Models", &stats.models, "models", 2, theme, cx))
            })
            .into_any_element()
    }

    fn ranking_section(
        &self,
        title: &'static str,
        rows: &[UsageRank],
        id_prefix: &'static str,
        metric_index: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let requested_metric = self.ranking_metrics[metric_index];
        let has_tokens = rows.iter().any(|row| row.has_token_data);
        let metric = if requested_metric == RankingMetric::Tokens && !has_tokens {
            RankingMetric::Interactions
        } else {
            requested_metric
        };
        let mut rows = rows.to_vec();
        rows.retain(|row| {
            metric != RankingMetric::Tokens || metric_index == 0 || row.has_token_data
        });
        rows.sort_by(|left, right| {
            let left_value = metric.value(left).unwrap_or(0);
            let right_value = metric.value(right).unwrap_or(0);
            right_value
                .cmp(&left_value)
                .then_with(|| right.sessions.cmp(&left.sessions))
                .then_with(|| left.label.cmp(&right.label))
        });
        if metric_index != 0 {
            rows.truncate(6);
        }
        let max = rows
            .iter()
            .filter_map(|row| metric.value(row))
            .max()
            .unwrap_or(0)
            .max(1);
        let row_elements = rows.iter().map(|row| ranking_row(row, max, metric, theme));
        statistics_section(theme)
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(section_heading(theme, title, ""))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(4.0))
                            .when(has_tokens, |toggles| {
                                toggles.child(ranking_toggle(
                                    theme,
                                    "Token",
                                    metric == RankingMetric::Tokens,
                                    format!("stats-{id_prefix}-tokens"),
                                    cx.listener(move |this, _, _, cx| {
                                        this.ranking_metrics[metric_index] = RankingMetric::Tokens;
                                        cx.notify();
                                    }),
                                ))
                            })
                            .child(ranking_toggle(
                                theme,
                                "交互轮数",
                                metric == RankingMetric::Interactions,
                                format!("stats-{id_prefix}-interactions"),
                                cx.listener(move |this, _, _, cx| {
                                    this.ranking_metrics[metric_index] =
                                        RankingMetric::Interactions;
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .children(row_elements)
            .when(rows.is_empty(), |section| {
                section.child(quiet_text(theme, "暂无可显示的记录"))
            })
            .into_any_element()
    }
}

impl ScrollRailHost for StatisticsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for StatisticsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let body = match &self.snapshot {
            Loadable::Idle | Loadable::Loading => widgets::page_column()
                .child(Self::page_heading(&theme))
                .child(
                    widgets::section_card(&theme)
                        .p(px(24.0))
                        .child(quiet_text(&theme, "正在汇总本机统计数据…")),
                )
                .into_any_element(),
            Loadable::Error(error) => widgets::page_column()
                .child(Self::page_heading(&theme))
                .child(
                    widgets::section_card(&theme)
                        .p(px(20.0))
                        .gap(px(12.0))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(13.0))
                                .text_color(theme.danger)
                                .child(SharedString::from(error.clone())),
                        ),
                )
                .into_any_element(),
            Loadable::Ready(stats) => {
                let page = widgets::page_column().child(Self::page_heading(&theme));
                if stats.totals.sessions == 0 {
                    page.child(
                        widgets::section_card(&theme)
                            .mt(px(24.0))
                            .p(px(22.0))
                            .gap(px(8.0))
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(14.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(SharedString::from("还没有可统计的会话")),
                            )
                            .child(quiet_text(
                                &theme,
                                "扫描本机 Agent 历史和 Zeron 会话后，统计会显示在这里。",
                            )),
                    )
                    .into_any_element()
                } else {
                    page.child(self.overview(stats, &theme))
                        .child(self.recent_comparison(stats, &theme))
                        .child(self.activity_heatmap(stats, &theme))
                        .child(self.agent_trend(stats, &theme))
                        .child(self.distribution(stats, &theme, cx))
                        .child(self.rankings(stats, &theme, cx))
                        .into_any_element()
                }
            }
        };

        let scrollbar = popover::rail(self, "statistics-page-scrollbar", &theme, cx);
        div()
            .id("statistics-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                div()
                    .id("statistics-page")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll.scroll)
                    .child(body),
            )
            .children(scrollbar)
    }
}

fn section_heading(theme: &Theme, title: &str, subtitle: &str) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .child(
            div()
                .text_size(crate::typography::ui_rems(13.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(SharedString::from(title.to_owned())),
        )
        .when(!subtitle.is_empty(), |heading| {
            heading.child(
                div()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(subtitle.to_owned())),
            )
        })
}

fn quiet_text(theme: &Theme, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_muted)
        .child(text.into())
}

fn interaction_total(days: &[UsageDay]) -> u64 {
    days.iter()
        .fold(0, |total, day| total.saturating_add(day.prompts))
}

fn distribution_buckets(
    distribution: &UsageDistribution,
    kind: DistributionKind,
) -> &[UsageBucket] {
    match kind {
        DistributionKind::Hours => &distribution.hours,
        DistributionKind::Weekdays => &distribution.weekdays,
        DistributionKind::Months => &distribution.months,
    }
}

fn active_days(days: &[UsageDay]) -> u64 {
    days.iter().filter(|day| day.prompts > 0).count() as u64
}

fn date_range(days: &[UsageDay]) -> String {
    let Some(first) = days.first() else {
        return "暂无日期".into();
    };
    let last = days.last().unwrap_or(first);
    format!("{}–{}", compact_date(&first.day), compact_date(&last.day))
}

fn compact_date(day: &str) -> String {
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|date| format!("{:02}/{:02}", date.month(), date.day()))
        .unwrap_or_else(|_| day.to_owned())
}

fn format_day_label(day: &str) -> String {
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|date| {
            format!(
                "{} {:02}/{:02}",
                weekday_label(date.weekday()),
                date.month(),
                date.day()
            )
        })
        .unwrap_or_else(|_| day.to_owned())
}

fn weekday_label(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "周一",
        Weekday::Tue => "周二",
        Weekday::Wed => "周三",
        Weekday::Thu => "周四",
        Weekday::Fri => "周五",
        Weekday::Sat => "周六",
        Weekday::Sun => "周日",
    }
}

fn heatmap_month_labels(weeks: &[Vec<Option<&UsageDay>>]) -> Vec<(usize, String)> {
    weeks
        .iter()
        .enumerate()
        .filter_map(|(index, week)| {
            let date = week.iter().flatten().find_map(|day| {
                let date = NaiveDate::parse_from_str(day.day.as_str(), "%Y-%m-%d").ok()?;
                (date.day() == 1).then_some(date)
            });
            let date = date.or_else(|| {
                if index == 0 {
                    week.iter().flatten().next().and_then(|day| {
                        NaiveDate::parse_from_str(day.day.as_str(), "%Y-%m-%d").ok()
                    })
                } else {
                    None
                }
            })?;
            Some((index, format!("{}月", date.month())))
        })
        .collect()
}

fn trend_month_labels(weeks: &[UsageTrendWeek]) -> Vec<(usize, String)> {
    let mut labels = Vec::new();
    let mut previous_month = None;
    for (index, week) in weeks.iter().enumerate() {
        let Ok(date) = NaiveDate::parse_from_str(&week.week_start, "%Y-%m-%d") else {
            continue;
        };
        let month = (date.year(), date.month());
        if previous_month != Some(month) {
            labels.push((index, format!("{}月", date.month())));
            previous_month = Some(month);
        }
    }
    labels
}

fn trend_agents(trend: &[UsageTrendWeek]) -> Vec<(String, String)> {
    let mut totals = std::collections::BTreeMap::<String, (String, u64)>::new();
    for week in trend {
        for agent in &week.agents {
            let entry = totals
                .entry(agent.agent_id.clone())
                .or_insert_with(|| (agent.agent_label.clone(), 0));
            entry.1 = entry.1.saturating_add(agent.prompts);
        }
    }
    let mut agents = totals
        .into_iter()
        .map(|(id, (label, prompts))| (id, label, prompts))
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));
    let has_other = agents.len() > 5;
    agents.truncate(5);
    let mut result = agents
        .into_iter()
        .map(|(id, label, _)| (id, label))
        .collect::<Vec<_>>();
    if has_other {
        result.push((TREND_OTHER_ID.into(), "其他".into()));
    }
    result
}

fn heatmap_legend(theme: &Theme) -> gpui::Div {
    let cells = (0..5).map(|index| {
        div()
            .size(px(HEATMAP_LEGEND_CELL_SIZE))
            .rounded(px(2.0))
            .bg(theme.accent.opacity(0.12 + index as f32 * 0.18))
    });
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(3.0))
        .child(quiet_text(theme, "少"))
        .children(cells)
        .child(quiet_text(theme, "多"))
}

fn heat_cell(day: &UsageDay, max_prompts: u64, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let opacity = if day.prompts == 0 {
        0.07
    } else if max_prompts == 0 {
        0.25
    } else {
        (0.2 + 0.68 * day.prompts as f32 / max_prompts as f32).clamp(0.2, 0.88)
    };
    let token_label = day
        .tokens
        .map(format_heatmap_tokens)
        .map(|tokens| format!("Token 消耗 {tokens}"))
        .unwrap_or_else(|| "Token 消耗暂无日期明细".into());
    let date_label = format!(
        "{} · {} 轮交互 · {}",
        format_day_label(&day.day),
        format_count(day.prompts),
        token_label
    );
    div()
        .id(SharedString::from(format!("stats-day-{}", day.day)))
        .size(px(HEATMAP_CELL_SIZE))
        .rounded(px(2.0))
        .bg(theme.accent.opacity(opacity))
        .tooltip(move |_, cx| {
            let text = date_label.clone();
            cx.new(|_| StatisticsTooltip { text }).into()
        })
}

struct StatisticsTooltip {
    text: String,
}

impl Render for StatisticsTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        div()
            .max_w(px(280.0))
            .px(px(10.0))
            .py(px(7.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.card_glass_bg())
            .text_size(crate::typography::ui_rems(11.0))
            .text_color(theme.text)
            .child(SharedString::from(self.text.clone()))
    }
}

fn change_color(now: u64, before: u64, theme: &Theme) -> Hsla {
    if now > before {
        theme.success
    } else if now < before {
        theme.text_muted
    } else {
        theme.text_faint
    }
}

fn agent_color(theme: &Theme, agent_id: &str) -> Hsla {
    let brand_color = match agent_id {
        "claude-code" => Some(0xD97757),
        "codex" => Some(0x7A8DFF),
        "grok" => Some(0x8A7F73),
        "dsh" => Some(0x4D6BFE),
        "cursor" => Some(0x2DB6C8),
        "opencode" => Some(0xC98A2E),
        "pi" => Some(0x3AAE8C),
        "omp" => Some(0xB05CE6),
        "kiro" => Some(0x9148FF),
        "kimi" => Some(0xE4739E),
        "gemini" => Some(0x3B8BD9),
        "copilot" => Some(0x6E9C3F),
        "antigravity" => Some(0x648AB5),
        "qoder" => Some(0x2BB454),
        "hermes" => Some(0xE0B040),
        "openclaw" => Some(0xE04A4A),
        "codebuddy" => Some(0x6C4DFF),
        "workbuddy" => Some(0x0EC8A9),
        "zcode" => Some(0x8B95A5),
        TREND_OTHER_ID => return theme.text_muted.opacity(0.35),
        _ => None,
    };
    brand_color.map_or(theme.accent, |color| gpui::rgb(color).into())
}

fn ranking_toggle(
    theme: &Theme,
    label: &'static str,
    selected: bool,
    id: String,
    click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    toggle_button(theme, label, selected, id, click)
}

fn distribution_toggle(
    theme: &Theme,
    label: &'static str,
    selected: bool,
    id: &'static str,
    click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    toggle_button(theme, label, selected, id, click)
}

fn toggle_button(
    theme: &Theme,
    label: &'static str,
    selected: bool,
    id: impl Into<SharedString>,
    click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .px(px(8.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .bg(if selected {
            theme.glass_hover()
        } else {
            theme.card_glass_bg()
        })
        .text_size(crate::typography::ui_rems(10.0))
        .text_color(if selected {
            theme.text
        } else {
            theme.text_muted
        })
        .cursor_pointer()
        .on_click(click)
        .child(SharedString::from(label))
}

fn ranking_row(rank: &UsageRank, max: u64, metric: RankingMetric, theme: &Theme) -> AnyElement {
    let value = metric.value(rank);
    let width = value
        .map(|value| (value as f32 / max as f32 * 100.0).clamp(0.0, 100.0))
        .unwrap_or(0.0);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .w(px(150.0))
                .min_w_0()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .truncate()
                .child(SharedString::from(rank.label.clone())),
        )
        .child(
            div()
                .flex_1()
                .h(px(7.0))
                .rounded(px(4.0))
                .bg(theme.accent.opacity(0.1))
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(width / 100.0))
                        .rounded(px(4.0))
                        .bg(agent_color(theme, &rank.id)),
                ),
        )
        .child(
            div()
                .w(px(72.0))
                .text_right()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(if value.is_some() {
                    theme.text
                } else {
                    theme.text_faint
                })
                .child(SharedString::from(
                    value
                        .map(|value| metric.format(value))
                        .unwrap_or_else(|| "—".into()),
                )),
        )
        .into_any_element()
}

fn percent_change(now: u64, before: u64) -> String {
    if before == 0 {
        return if now == 0 {
            "无活动".into()
        } else {
            "新增".into()
        };
    }
    let change = ((now as f64 - before as f64) / before as f64 * 100.0).round() as i64;
    format!("{:+}%", change)
}

fn statistics_section(theme: &Theme) -> gpui::Div {
    div()
        .w_full()
        .mt(px(24.0))
        .pb(px(18.0))
        .border_b_1()
        .border_color(theme.border)
        .flex()
        .flex_col()
}

fn format_count(value: u64) -> String {
    let raw = value.to_string();
    let mut formatted = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, character) in raw.chars().enumerate() {
        if index > 0 && (raw.len() - index) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(character);
    }
    formatted
}

fn format_tokens(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 10_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        format_count(value)
    }
}

fn format_heatmap_tokens(value: u64) -> String {
    let (divisor, suffix) = if value >= 999_500_000 {
        (1_000_000_000.0, "B")
    } else if value >= 999_500 {
        (1_000_000.0, "M")
    } else if value >= 1_000 {
        (1_000.0, "K")
    } else {
        return format_count(value);
    };

    let rounded = (value as f64 / divisor * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}{suffix}")
    } else {
        format!("{rounded:.1}{suffix}")
    }
}
