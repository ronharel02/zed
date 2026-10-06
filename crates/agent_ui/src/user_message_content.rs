use std::{ops::Range, sync::Arc};

use acp_thread::{AcpThread, MentionUri};
use agent_client_protocol::schema::v2 as acp;
use gpui::{App, Entity, ImageFormat};
use text::LineEnding;
use util::{ResultExt as _, paths::PathStyle};

use crate::mention_set::{Mention, MentionImage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UserMessageContent {
    text: Arc<str>,
    mentions: Vec<UserMessageMention>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UserMessageMention {
    pub(crate) range: Range<usize>,
    pub(crate) uri: MentionUri,
    pub(crate) content: Mention,
    label: String,
    label_range: Option<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UserMessageContentSegment {
    Text(String),
    Mention { uri: MentionUri, label: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UserMessageContentLineSegment {
    pub(crate) content: UserMessageContentSegment,
    pub(crate) source_range: Option<Range<usize>>,
}

impl UserMessageContentLineSegment {
    pub(crate) fn display_text(&self) -> &str {
        match &self.content {
            UserMessageContentSegment::Text(text) => text,
            UserMessageContentSegment::Mention { label, .. } => label,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UserMessageContentLine {
    message_text: Arc<str>,
    source_range: Option<Range<usize>>,
    pub(crate) segments: Vec<UserMessageContentLineSegment>,
    pub(crate) has_more_content: bool,
}

impl UserMessageContentLine {
    pub(crate) fn message_text(&self) -> &Arc<str> {
        &self.message_text
    }

    pub(crate) fn source_range(&self) -> Option<&Range<usize>> {
        self.source_range.as_ref()
    }
}

impl UserMessageContent {
    pub(crate) fn for_thread_entry(
        thread: &Entity<AcpThread>,
        entry_index: usize,
        cx: &App,
    ) -> Option<Self> {
        let thread = thread.read(cx);
        let path_style = thread.project().read(cx).path_style(cx);
        let blocks = thread
            .entries()
            .get(entry_index)?
            .user_message()?
            .content
            .source_blocks()
            .to_vec();
        Some(Self::from_source_blocks(blocks, path_style))
    }

    pub(crate) fn from_source_blocks(
        mut blocks: Vec<acp::ContentBlock>,
        path_style: PathStyle,
    ) -> Self {
        if !blocks.iter().all(acp_thread::content::can_convert_to_v1) {
            blocks.retain(|block| {
                acp_thread::content::can_convert_to_v1(block)
                    || matches!(block, acp::ContentBlock::Text(_))
            });
            blocks.push(acp::ContentBlock::Text(acp::TextContent::new(
                "\n[Unsupported message content — this message cannot be edited or resent]",
            )));
        }
        Self::from_blocks(blocks, path_style)
    }

    pub(crate) fn from_blocks(
        blocks: impl IntoIterator<Item = acp::ContentBlock>,
        path_style: PathStyle,
    ) -> Self {
        let mut text = String::new();
        let mut mentions = Vec::new();

        let append_text = |text: &mut String, mut segment: String| {
            LineEnding::normalize(&mut segment);
            text.push_str(&segment);
        };
        let append_mention = |text: &mut String,
                              mentions: &mut Vec<UserMessageMention>,
                              uri: MentionUri,
                              content: Mention| {
            let mut label = format!("@{}", uri.name());
            LineEnding::normalize(&mut label);
            let mut serialized = uri.as_link().to_string();
            LineEnding::normalize(&mut serialized);
            let start = text.len();
            let label_range = serialized
                .find(&label)
                .map(|label_start| start + label_start..start + label_start + label.len());
            text.push_str(&serialized);
            mentions.push(UserMessageMention {
                range: start..text.len(),
                uri,
                content,
                label,
                label_range,
            });
        };

        for block in blocks {
            match block {
                acp::ContentBlock::Text(text_content) => {
                    append_text(&mut text, text_content.text);
                }
                acp::ContentBlock::Resource(acp::EmbeddedResource {
                    resource: acp::EmbeddedResourceResource::TextResourceContents(resource),
                    ..
                }) => {
                    let Some(uri) = MentionUri::parse(&resource.uri, path_style).log_err() else {
                        append_text(&mut text, resource.uri);
                        continue;
                    };
                    append_mention(
                        &mut text,
                        &mut mentions,
                        uri,
                        Mention::Text {
                            content: resource.text,
                            tracked_buffers: Vec::new(),
                        },
                    );
                }
                acp::ContentBlock::ResourceLink(resource) => {
                    let Some(uri) = MentionUri::parse(&resource.uri, path_style).log_err() else {
                        append_text(&mut text, format!("@{}", resource.name));
                        continue;
                    };
                    append_mention(&mut text, &mut mentions, uri, Mention::Link);
                }
                acp::ContentBlock::Image(acp::ImageContent {
                    uri,
                    data,
                    mime_type,
                    ..
                }) => {
                    let mention_uri = if let Some(uri) = uri.as_deref() {
                        MentionUri::parse(uri, path_style)
                    } else {
                        Ok(MentionUri::PastedImage {
                            name: "Image".to_string(),
                        })
                    };
                    let Some(uri) = mention_uri.log_err() else {
                        if let Some(uri) = uri {
                            append_text(&mut text, uri);
                        }
                        continue;
                    };
                    let Some(format) = ImageFormat::from_mime_type(&mime_type.0) else {
                        log::error!("failed to parse MIME type for image: {mime_type:?}");
                        continue;
                    };
                    append_mention(
                        &mut text,
                        &mut mentions,
                        uri,
                        Mention::Image(MentionImage {
                            data: data.into(),
                            format,
                        }),
                    );
                }
                _ => {}
            }
        }

        Self {
            text: text.into(),
            mentions,
        }
    }

    pub(crate) fn into_parts(self) -> (Arc<str>, Vec<UserMessageMention>) {
        (self.text, self.mentions)
    }

    pub(crate) fn text(&self) -> &Arc<str> {
        &self.text
    }

    pub(crate) fn has_same_projection(&self, other: &Self) -> bool {
        self.text == other.text
            && self
                .mentions
                .iter()
                .map(|mention| (&mention.range, &mention.uri))
                .eq(other
                    .mentions
                    .iter()
                    .map(|mention| (&mention.range, &mention.uri)))
    }

    pub(crate) fn first_line(&self) -> UserMessageContentLine {
        let Some((line_range, has_more_content)) = self.first_non_empty_line() else {
            return UserMessageContentLine {
                message_text: self.text.clone(),
                source_range: None,
                segments: vec![UserMessageContentLineSegment {
                    content: UserMessageContentSegment::Text("Message".to_string()),
                    source_range: None,
                }],
                has_more_content: false,
            };
        };

        let mut segments = Vec::new();
        let mut cursor = line_range.start;
        for mention in self
            .mentions
            .iter()
            .filter(|mention| mention.range.start >= line_range.start)
            .take_while(|mention| mention.range.end <= line_range.end)
        {
            if cursor < mention.range.start {
                push_text_segment(&self.text, cursor..mention.range.start, &mut segments);
            }
            segments.push(UserMessageContentLineSegment {
                content: UserMessageContentSegment::Mention {
                    uri: mention.uri.clone(),
                    label: mention.label.clone(),
                },
                source_range: mention.label_range.clone(),
            });
            cursor = mention.range.end;
        }
        if cursor < line_range.end {
            push_text_segment(&self.text, cursor..line_range.end, &mut segments);
        }

        UserMessageContentLine {
            message_text: self.text.clone(),
            source_range: Some(line_range),
            segments,
            has_more_content,
        }
    }

    fn first_non_empty_line(&self) -> Option<(Range<usize>, bool)> {
        let mut first_line = None;
        let mut offset = 0;

        for line in self.text.split_inclusive('\n') {
            let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
            let trimmed_start =
                line_without_newline.len() - line_without_newline.trim_start().len();
            let trimmed_end = line_without_newline.trim_end().len();
            if trimmed_start < trimmed_end {
                let range = offset + trimmed_start..offset + trimmed_end;
                if first_line.is_some() {
                    return first_line.map(|first_line| (first_line, true));
                }
                first_line = Some(range);
            }
            offset += line.len();
        }

        first_line.map(|first_line| (first_line, false))
    }
}

fn push_text_segment(
    text: &str,
    range: Range<usize>,
    segments: &mut Vec<UserMessageContentLineSegment>,
) {
    let raw_text = &text[range.clone()];
    let trimmed_start = raw_text.len() - raw_text.trim_start().len();
    let trimmed_end = raw_text.trim_end().len();
    if trimmed_start >= trimmed_end {
        return;
    }

    segments.push(UserMessageContentLineSegment {
        content: UserMessageContentSegment::Text(raw_text[trimmed_start..trimmed_end].to_string()),
        source_range: Some(range.start + trimmed_start..range.start + trimmed_end),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_content_filters_unsupported_blocks_and_warns() {
        let blocks = vec![
            acp::ContentBlock::Text(acp::TextContent::new("visible\r\ntext ")),
            acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
                "main.rs",
                "file:///project/main.rs",
            )),
            acp::ContentBlock::ResourceLink(
                acp::ResourceLink::new("hidden.rs", "file:///project/hidden.rs").annotations(
                    acp::Annotations::new().audience(vec![acp::Role::Other("_future".into())]),
                ),
            ),
            acp::ContentBlock::Other(acp::OtherContentBlock::new("_future", Default::default())),
        ];

        let content = UserMessageContent::from_source_blocks(blocks.clone(), PathStyle::Unix);
        assert_eq!(
            content.text.as_ref(),
            "visible\ntext [@main.rs](file:///project/main.rs)\n[Unsupported message content — this message cannot be edited or resent]"
        );
        assert_eq!(content.mentions.len(), 1);
        assert!(content.first_line().has_more_content);

        let draft = UserMessageContent::from_blocks(blocks, PathStyle::Unix);
        assert_eq!(draft.mentions.len(), 2);
        assert!(!draft.text.contains("Unsupported message content"));
    }

    #[test]
    fn source_content_preserves_unsupported_text_annotations_and_warns() {
        let block = acp::ContentBlock::Text(acp::TextContent::new("annotated text").annotations(
            acp::Annotations::new().audience(vec![acp::Role::Other("_future".into())]),
        ));
        let content = UserMessageContent::from_source_blocks(vec![block], PathStyle::Unix);
        assert_eq!(
            content.text.as_ref(),
            "annotated text\n[Unsupported message content — this message cannot be edited or resent]"
        );

        let supported = vec![acp::ContentBlock::Text(acp::TextContent::new("known text"))];
        assert_eq!(
            UserMessageContent::from_source_blocks(supported.clone(), PathStyle::Unix),
            UserMessageContent::from_blocks(supported, PathStyle::Unix)
        );
    }

    #[test]
    fn first_line_uses_the_same_normalized_mentions_as_the_editor() {
        let content = UserMessageContent::from_blocks(
            vec![
                acp::ContentBlock::Text(acp::TextContent::new("Check ")),
                acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
                    "main.rs",
                    "file:///project/main.rs",
                )),
                acp::ContentBlock::Text(acp::TextContent::new(" now")),
            ],
            PathStyle::Unix,
        );

        assert_eq!(content.mentions.len(), 1);
        assert_eq!(
            &content.text[content.mentions[0].range.clone()],
            "[@main.rs](file:///project/main.rs)"
        );
        assert!(matches!(
            content.first_line().segments.as_slice(),
            [
                UserMessageContentLineSegment {
                    content: UserMessageContentSegment::Text(_),
                    ..
                },
                UserMessageContentLineSegment {
                    content: UserMessageContentSegment::Mention { .. },
                    ..
                },
                UserMessageContentLineSegment {
                    content: UserMessageContentSegment::Text(_),
                    ..
                }
            ]
        ));
        assert_eq!(
            content
                .first_line()
                .segments
                .into_iter()
                .map(|segment| segment.source_range)
                .collect::<Vec<_>>(),
            vec![Some(0..5), Some(7..15), Some(42..45)]
        );
        assert_eq!(content.first_line().source_range, Some(0..45));
    }

    #[test]
    fn first_line_uses_image_label_instead_of_serialized_link() {
        let content = UserMessageContent::from_blocks(
            vec![
                acp::ContentBlock::Image(
                    acp::ImageContent::new("ignored", "image/png")
                        .uri("zed:///agent/pasted-image?name=Diagram"),
                ),
                acp::ContentBlock::Text(acp::TextContent::new("\nExplain this diagram")),
            ],
            PathStyle::Unix,
        );

        let line = content.first_line();
        assert!(matches!(
            line.segments.as_slice(),
            [UserMessageContentLineSegment {
                content: UserMessageContentSegment::Mention { label, .. },
                ..
            }] if label == "@Diagram"
        ));
        assert!(line.has_more_content);
    }

    #[test]
    fn first_line_uses_the_first_non_empty_line() {
        let content = UserMessageContent::from_blocks(
            vec![acp::ContentBlock::Text(acp::TextContent::new(
                "\n  First line  \n\nSecond line",
            ))],
            PathStyle::Unix,
        );

        let line = content.first_line();
        assert_eq!(line.segments.len(), 1);
        assert_eq!(line.segments[0].display_text(), "First line");
        assert_eq!(line.segments[0].source_range, Some(3..13));
        assert_eq!(line.source_range, Some(3..13));
        assert!(line.has_more_content);
    }

    #[test]
    fn first_line_falls_back_for_empty_content() {
        let content = UserMessageContent::from_blocks(
            vec![acp::ContentBlock::Text(acp::TextContent::new("\n   \n"))],
            PathStyle::Unix,
        );

        let line = content.first_line();
        assert_eq!(line.segments.len(), 1);
        assert_eq!(line.segments[0].display_text(), "Message");
        assert_eq!(line.segments[0].source_range, None);
        assert_eq!(line.source_range, None);
        assert!(!line.has_more_content);
    }

    #[test]
    fn invalid_resource_links_fall_back_to_normalized_labels() {
        let content = UserMessageContent::from_blocks(
            vec![acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
                "notes.md\r\nMore context",
                "not a valid uri",
            ))],
            PathStyle::Unix,
        );

        assert_eq!(content.text.as_ref(), "@notes.md\nMore context");
        assert!(content.mentions.is_empty());
        let line = content.first_line();
        assert_eq!(line.segments.len(), 1);
        assert_eq!(line.segments[0].display_text(), "@notes.md");
        assert_eq!(line.segments[0].source_range, Some(0..9));
        assert!(line.has_more_content);
    }

    #[test]
    fn invalid_embedded_resources_fall_back_to_the_uri() {
        let content = UserMessageContent::from_blocks(
            vec![acp::ContentBlock::Resource(acp::EmbeddedResource::new(
                acp::EmbeddedResourceResource::TextResourceContents(
                    acp::TextResourceContents::new("contents", "not a valid uri"),
                ),
            ))],
            PathStyle::Unix,
        );

        assert_eq!(content.text.as_ref(), "not a valid uri");
        assert!(content.mentions.is_empty());
        let line = content.first_line();
        assert_eq!(line.segments[0].display_text(), "not a valid uri");
        assert_eq!(line.segments[0].source_range, Some(0..15));
    }

    #[test]
    fn invalid_image_uris_fall_back_to_text() {
        let content = UserMessageContent::from_blocks(
            vec![acp::ContentBlock::Image(
                acp::ImageContent::new("ignored", "image/png").uri("not a valid uri"),
            )],
            PathStyle::Unix,
        );

        assert_eq!(content.text.as_ref(), "not a valid uri");
        assert!(content.mentions.is_empty());
        let line = content.first_line();
        assert_eq!(line.segments[0].display_text(), "not a valid uri");
        assert_eq!(line.segments[0].source_range, Some(0..15));
    }

    #[test]
    fn first_line_omits_whitespace_between_mentions() {
        let content = UserMessageContent::from_blocks(
            vec![
                acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
                    "main.rs",
                    "file:///project/main.rs",
                )),
                acp::ContentBlock::Text(acp::TextContent::new(" \t ")),
                acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
                    "lib.rs",
                    "file:///project/lib.rs",
                )),
            ],
            PathStyle::Unix,
        );

        let line = content.first_line();
        assert_eq!(line.segments.len(), 2);
        assert!(
            line.segments.iter().all(|segment| matches!(
                segment.content,
                UserMessageContentSegment::Mention { .. }
            ))
        );
    }
}
