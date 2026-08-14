use std::ops::Range;

use gpui::{AnyElement, App, AvailableSpace, HighlightStyle, Pixels, StyledText, Window};
use ui::{LabelLike, prelude::*};

use crate::user_message_content::{UserMessageContentLineSegment, UserMessageContentSegment};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StickyUserMessageSearchHighlight {
    range: Range<usize>,
    is_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StickyUserMessageSearchHighlights {
    segment_ranges: Vec<Vec<StickyUserMessageSearchHighlight>>,
}

pub(crate) fn sticky_user_message_search_highlights(
    segments: &[UserMessageContentLineSegment],
    matches: impl IntoIterator<Item = (Range<usize>, bool)>,
) -> Option<StickyUserMessageSearchHighlights> {
    let mut segment_ranges = None;
    for (match_range, is_active) in matches {
        if match_range.is_empty() {
            continue;
        }
        let Some((segment_index, segment, source_range)) =
            segments.iter().enumerate().find_map(|(index, segment)| {
                let source_range = segment.source_range.as_ref()?;
                (match_range.start >= source_range.start && match_range.end <= source_range.end)
                    .then_some((index, segment, source_range))
            })
        else {
            continue;
        };

        let display_range =
            match_range.start - source_range.start..match_range.end - source_range.start;
        let display_text = segment.display_text();
        if display_range.end > display_text.len()
            || !display_text.is_char_boundary(display_range.start)
            || !display_text.is_char_boundary(display_range.end)
        {
            continue;
        }

        segment_ranges.get_or_insert_with(|| vec![Vec::new(); segments.len()])[segment_index].push(
            StickyUserMessageSearchHighlight {
                range: display_range,
                is_active,
            },
        );
    }

    Some(StickyUserMessageSearchHighlights {
        segment_ranges: segment_ranges?,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StickyUserMessageFit {
    pub(crate) visible_segment_count: usize,
    pub(crate) show_ellipsis: bool,
}

fn fit_sticky_user_message_segments(
    segment_widths: &[Pixels],
    gap_width: Pixels,
    ellipsis_width: Pixels,
    available_width: Pixels,
    has_more_message_content: bool,
) -> StickyUserMessageFit {
    let mut visible_segment_count = segment_widths.len();
    let mut show_ellipsis = has_more_message_content;

    if segment_widths.len() > 1 {
        loop {
            show_ellipsis =
                has_more_message_content || visible_segment_count < segment_widths.len();
            if visible_segment_count == 0
                || sticky_user_message_preview_width(
                    segment_widths,
                    visible_segment_count,
                    gap_width,
                    ellipsis_width,
                    show_ellipsis,
                ) <= available_width
            {
                break;
            }
            visible_segment_count -= 1;
        }
    }

    StickyUserMessageFit {
        visible_segment_count,
        show_ellipsis,
    }
}

fn sticky_user_message_preview_width(
    segment_widths: &[Pixels],
    visible_segment_count: usize,
    gap_width: Pixels,
    ellipsis_width: Pixels,
    show_ellipsis: bool,
) -> Pixels {
    let segment_width = segment_widths
        .iter()
        .take(visible_segment_count)
        .fold(Pixels::ZERO, |sum, width| sum + *width);
    let child_count = visible_segment_count + usize::from(show_ellipsis);
    let gap_width = if child_count > 1 {
        gap_width * (child_count - 1) as f32
    } else {
        Pixels::ZERO
    };
    let ellipsis_width = if show_ellipsis {
        ellipsis_width
    } else {
        Pixels::ZERO
    };

    segment_width + gap_width + ellipsis_width
}

pub(crate) fn render_sticky_user_message_preview(
    segments: Vec<UserMessageContentLineSegment>,
    search_highlights: Option<StickyUserMessageSearchHighlights>,
    has_more_message_content: bool,
    available_width: Pixels,
    rem_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let render_text = |text: String,
                       highlights: Option<&Vec<StickyUserMessageSearchHighlight>>,
                       truncate: bool,
                       cx: &mut App| {
        let Some(highlights) = highlights.filter(|highlights| !highlights.is_empty()) else {
            return Label::new(text)
                .size(LabelSize::Small)
                .color(Color::Default)
                .map(|this| {
                    if truncate {
                        this.truncate()
                    } else {
                        this.single_line().flex_none()
                    }
                })
                .into_any_element();
        };

        let colors = cx.theme().colors();
        let highlights = highlights.iter().map(|highlight| {
            (
                highlight.range.clone(),
                HighlightStyle {
                    background_color: Some(if highlight.is_active {
                        colors.search_active_match_background
                    } else {
                        colors.search_match_background
                    }),
                    ..Default::default()
                },
            )
        });
        let label = LabelLike::new()
            .size(LabelSize::Small)
            .color(Color::Default)
            .map(|this| {
                if truncate {
                    this.truncate()
                } else {
                    this.single_line()
                }
            })
            .child(StyledText::new(text).with_highlights(highlights));

        if truncate {
            div().min_w_0().child(label).into_any_element()
        } else {
            div().flex_none().child(label).into_any_element()
        }
    };
    let render_segment = |index: usize,
                          segment: UserMessageContentLineSegment,
                          truncate: bool,
                          cx: &mut App| {
        let highlights = search_highlights
            .as_ref()
            .and_then(|highlights| highlights.segment_ranges.get(index));
        match segment.content {
            UserMessageContentSegment::Text(text) => render_text(text, highlights, truncate, cx),
            UserMessageContentSegment::Mention { uri, label } => h_flex()
                .id(("sticky-user-message-mention", index))
                .flex_none()
                .h_5()
                .px_1p5()
                .gap_1()
                .items_center()
                .rounded_sm()
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .bg(cx.theme().colors().element_background)
                .child(
                    Icon::from_path(uri.icon_path(cx))
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(render_text(label, highlights, false, cx))
                .into_any_element(),
        }
    };
    let render_ellipsis = || {
        Label::new("…")
            .size(LabelSize::Small)
            .color(Color::Muted)
            .flex_shrink_0()
            .into_any_element()
    };
    let measure = |element: &mut AnyElement, window: &mut Window, cx: &mut App| {
        window.with_rem_size(Some(rem_size), |window| {
            element
                .layout_as_root(AvailableSpace::min_size(), window, cx)
                .width
        })
    };

    let (ellipsis_width, segment_widths) = if segments.len() > 1 {
        let mut ellipsis = render_ellipsis();
        let ellipsis_width = measure(&mut ellipsis, window, cx);
        let segment_widths = segments
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, segment)| {
                let mut element = render_segment(index, segment, false, cx);
                measure(&mut element, window, cx)
            })
            .collect();
        (ellipsis_width, segment_widths)
    } else {
        (Pixels::ZERO, vec![Pixels::ZERO; segments.len()])
    };
    let fit = fit_sticky_user_message_segments(
        &segment_widths,
        rems(0.25).to_pixels(rem_size),
        ellipsis_width,
        available_width,
        has_more_message_content,
    );

    let include_dropped_text = segments
        .get(fit.visible_segment_count)
        .is_some_and(|segment| matches!(segment.content, UserMessageContentSegment::Text(_)));
    let visible_segment_count = fit.visible_segment_count + usize::from(include_dropped_text);
    let truncated_text_index = if include_dropped_text {
        Some(fit.visible_segment_count)
    } else if segments.len() == 1
        && matches!(
            segments.first(),
            Some(UserMessageContentLineSegment {
                content: UserMessageContentSegment::Text(_),
                ..
            })
        )
    {
        Some(0)
    } else {
        None
    };

    let mut rendered_segments = segments
        .into_iter()
        .take(visible_segment_count)
        .enumerate()
        .map(|(index, segment)| {
            let truncate = Some(index) == truncated_text_index;
            render_segment(index, segment, truncate, cx)
        })
        .collect::<Vec<_>>();
    let show_ellipsis = !include_dropped_text
        && (fit.show_ellipsis || visible_segment_count < segment_widths.len());
    if show_ellipsis {
        rendered_segments.push(render_ellipsis());
    }

    h_flex()
        .min_w_0()
        .flex_1()
        .overflow_hidden()
        .gap_1()
        .items_center()
        .children(rendered_segments)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_segment(text: &str, source_range: Range<usize>) -> UserMessageContentLineSegment {
        UserMessageContentLineSegment {
            content: UserMessageContentSegment::Text(text.to_string()),
            source_range: Some(source_range),
        }
    }

    #[test]
    fn fit_segments_keeps_everything_that_fits() {
        let fit = fit_sticky_user_message_segments(
            &[gpui::px(10.0), gpui::px(10.0)],
            gpui::px(1.0),
            gpui::px(3.0),
            gpui::px(21.0),
            false,
        );

        assert_eq!(fit.visible_segment_count, 2);
        assert!(!fit.show_ellipsis);
    }

    #[test]
    fn fit_segments_removes_trailing_segments_to_make_room_for_ellipsis() {
        let fit = fit_sticky_user_message_segments(
            &[gpui::px(10.0), gpui::px(10.0), gpui::px(10.0)],
            gpui::px(1.0),
            gpui::px(3.0),
            gpui::px(25.0),
            false,
        );

        assert_eq!(fit.visible_segment_count, 2);
        assert!(fit.show_ellipsis);
    }

    #[test]
    fn fit_segments_leaves_a_single_segment_for_flexible_truncation() {
        let fit = fit_sticky_user_message_segments(
            &[gpui::px(100.0)],
            gpui::px(1.0),
            gpui::px(3.0),
            gpui::px(10.0),
            true,
        );

        assert_eq!(fit.visible_segment_count, 1);
        assert!(fit.show_ellipsis);
    }

    #[test]
    fn search_highlights_are_mapped_to_display_segments() {
        let segments = [
            text_segment("first", 0..5),
            text_segment("reference", 6..15),
            text_segment("more content", 16..28),
        ];
        let highlights =
            sticky_user_message_search_highlights(&segments, [(7..11, false), (20..24, true)])
                .expect("expected matches in sticky preview");

        assert_eq!(highlights.segment_ranges[0], Vec::new());
        assert_eq!(highlights.segment_ranges[1].len(), 1);
        assert_eq!(highlights.segment_ranges[1][0].range, 1..5);
        assert!(!highlights.segment_ranges[1][0].is_active);
        assert_eq!(highlights.segment_ranges[2].len(), 1);
        assert_eq!(highlights.segment_ranges[2][0].range, 4..8);
        assert!(highlights.segment_ranges[2][0].is_active);
    }

    #[test]
    fn search_highlights_skip_empty_and_invalid_utf8_ranges() {
        let segments = [text_segment("éclair", 0..7)];

        assert_eq!(
            sticky_user_message_search_highlights(&segments, [(0..0, false), (1..3, true)]),
            None
        );
    }
}
