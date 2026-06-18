//! Chat message rendering — avatars, bubbles, markdown, tool calls, indicators.

use super::super::chat_types::{Attachment, ChatMessage, MessageRole};
use super::super::content_parser::{split_by_code_blocks, ContentPart};
use super::super::syntax_highlight::format_code_block;
use super::super::tool_calls::{ToolCall, ToolCallStatus};
use super::ChatPanel;
use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Stroke, Ui, Vec2};

impl ChatPanel {
    pub(super) fn render_message(
        &self,
        ui: &mut Ui,
        message: &mut ChatMessage,
        theme: &super::super::Theme,
        is_last: bool,
    ) {
        let is_user = matches!(message.role, MessageRole::User);

        // Full-width message container
        Frame::NONE.fill(theme.background_darkest()).show(ui, |ui| {
            ui.set_width(ui.available_width());

            // Message content with proper padding
            ui.horizontal(|ui| {
                // Left margin/avatar area
                ui.add_space(if is_user { 80.0 } else { 24.0 });

                // Message content
                ui.vertical(|ui| {
                    // Avatar and name row
                    ui.horizontal(|ui| {
                        if !is_user {
                            // Claude avatar
                            self.render_claude_avatar(ui, theme);
                            ui.add_space(12.0);

                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("wgenty")
                                        .size(14.0)
                                        .strong()
                                        .color(theme.primary_color()),
                                );

                                // Timestamp
                                ui.label(
                                    RichText::new(message.timestamp.format("%I:%M %p").to_string())
                                        .size(11.0)
                                        .color(theme.muted_text_color()),
                                );
                            });
                        } else {
                            // User info
                            ui.with_layout(Layout::right_to_left(egui::Align::Min), |ui| {
                                self.render_user_avatar(ui, theme);
                            });
                        }
                    });

                    ui.add_space(8.0);

                    // Message content or bubble
                    if is_user {
                        // User message - right aligned bubble
                        ui.with_layout(Layout::right_to_left(egui::Align::Min), |ui| {
                            self.render_user_message_bubble(ui, message, theme);
                        });
                    } else {
                        // Claude message - left aligned with full width content
                        self.render_claude_message_content(ui, message, theme, is_last);
                    }

                    // Tool calls for assistant messages
                    if !is_user && !message.tool_calls.is_empty() {
                        ui.add_space(16.0);
                        for tool_call in &message.tool_calls {
                            self.render_tool_call_card(ui, tool_call, theme);
                            ui.add_space(8.0);
                        }
                    }
                });

                // Right margin
                ui.add_space(24.0);
            });
        });

        ui.add_space(24.0);
    }

    fn render_claude_avatar(&self, ui: &mut Ui, theme: &super::super::Theme) {
        Frame::NONE
            .fill(theme.surface_color())
            .corner_radius(CornerRadius::same(8))
            .stroke(Stroke::new(1.0, theme.border_color()))
            .show(ui, |ui| {
                super::super::brand_icon::show(ui, Vec2::new(32.0, 32.0));
            });
    }

    fn render_user_avatar(&self, ui: &mut Ui, _theme: &super::super::Theme) {
        Frame::NONE
            .fill(Color32::from_rgb(80, 80, 80))
            .corner_radius(CornerRadius::same(8))
            .show(ui, |ui| {
                ui.set_width(32.0);
                ui.set_height(32.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("👤").size(18.0));
                });
            });
    }

    fn render_user_message_bubble(
        &self,
        ui: &mut Ui,
        message: &ChatMessage,
        theme: &super::super::Theme,
    ) {
        let max_width = 600.0f32.min(ui.available_width() * 0.8);

        Frame::NONE
            .fill(Color32::from_rgb(212, 165, 116)) // Claude orange
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_max_width(max_width);

                // Message text
                ui.label(
                    RichText::new(&message.content)
                        .size(15.0)
                        .color(Color32::WHITE),
                );

                // Attachments
                for attachment in &message.attachments {
                    ui.add_space(8.0);
                    self.render_attachment(ui, attachment, theme);
                }
            });
    }

    fn render_claude_message_content(
        &self,
        ui: &mut Ui,
        message: &mut ChatMessage,
        theme: &super::super::Theme,
        is_last: bool,
    ) {
        let max_width = 720.0f32.min(ui.available_width());

        ui.vertical(|ui| {
            ui.set_max_width(max_width);

            // Thinking process (expandable)
            if let Some(thinking) = &message.thinking {
                self.render_thinking_process(ui, thinking, &mut message.thinking_expanded, theme);
                ui.add_space(12.0);
            }

            // Main content
            let content_str = if message.is_streaming {
                format!("{}▌", message.content)
            } else {
                message.content.clone()
            };

            if message.content_collapsed && !message.is_streaming {
                // Collapsed: show preview + toggle button
                let preview_lines: Vec<&str> = message.content.lines().take(3).collect();
                let total_lines = message.content.lines().count();
                ui.horizontal(|ui| {
                    if ui
                        .button(
                            RichText::new(format!("▶ {} lines (collapsed)", total_lines))
                                .size(12.0)
                                .color(theme.muted_text_color()),
                        )
                        .clicked()
                    {
                        message.content_collapsed = false;
                    }
                });
                Frame::NONE
                    .fill(theme.surface_color())
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(12))
                    .stroke(Stroke::new(1.0, theme.border_color()))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for line in &preview_lines {
                            ui.label(RichText::new(*line).size(14.0).color(theme.text_color()));
                        }
                        ui.label(
                            RichText::new(format!(
                                "... ({} lines total, click to expand)",
                                total_lines
                            ))
                            .size(12.0)
                            .color(theme.muted_text_color())
                            .italics(),
                        );
                    });
            } else {
                // Expanded: show full content with collapse button
                if !message.is_streaming && message.content.lines().count() > 0 {
                    ui.horizontal(|ui| {
                        if ui
                            .button(
                                RichText::new("▼ Expanded")
                                    .size(12.0)
                                    .color(theme.muted_text_color()),
                            )
                            .clicked()
                        {
                            message.content_collapsed = true;
                        }
                    });
                }
                self.render_markdown_content(ui, &content_str, theme);
            }

            // Streaming cursor animation for last message
            if message.is_streaming && is_last {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    self.render_cursor_animation(ui);
                });
            }
        });
    }

    fn render_thinking_process(
        &self,
        ui: &mut Ui,
        thinking: &str,
        expanded: &mut bool,
        theme: &super::super::Theme,
    ) {
        let header_text = if *expanded {
            "▼ Thinking"
        } else {
            "▶ Thinking"
        };

        if ui
            .button(
                RichText::new(header_text)
                    .size(12.0)
                    .color(theme.muted_text_color()),
            )
            .clicked()
        {
            *expanded = !*expanded;
        }

        if *expanded {
            Frame::NONE
                .fill(Color32::from_rgb(30, 30, 35))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(12))
                .stroke(Stroke::new(1.0, Color32::from_rgb(60, 60, 70)))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(thinking)
                            .size(12.0)
                            .color(theme.muted_text_color())
                            .italics(),
                    );
                });
        }
    }

    fn render_markdown_content(&self, ui: &mut Ui, content: &str, theme: &super::super::Theme) {
        // Split content by code blocks
        let parts = split_by_code_blocks(content);
        let mut in_list = false;
        let mut list_number = 0;

        for part in parts {
            match part {
                ContentPart::Text(text) => {
                    // Process markdown line by line
                    for line in text.lines() {
                        let trimmed = line.trim();

                        if trimmed.is_empty() {
                            if in_list {
                                in_list = false;
                                list_number = 0;
                            }
                            ui.add_space(8.0);
                            continue;
                        }

                        // Headers
                        if let Some(header_text) = trimmed.strip_prefix("# ") {
                            ui.add_space(16.0);
                            ui.label(
                                RichText::new(header_text)
                                    .size(24.0)
                                    .strong()
                                    .color(theme.text_color()),
                            );
                            ui.add_space(8.0);
                        } else if let Some(header_text) = trimmed.strip_prefix("## ") {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(header_text)
                                    .size(20.0)
                                    .strong()
                                    .color(theme.text_color()),
                            );
                            ui.add_space(6.0);
                        } else if let Some(header_text) = trimmed.strip_prefix("### ") {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(header_text)
                                    .size(16.0)
                                    .strong()
                                    .color(theme.text_color()),
                            );
                            ui.add_space(4.0);
                        }
                        // Bullet lists
                        else if let Some(item) = trimmed.strip_prefix("- ") {
                            in_list = true;
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("•").size(16.0).color(theme.primary_color()),
                                );
                                ui.add_space(8.0);
                                ui.label(RichText::new(item).size(15.0).color(theme.text_color()));
                            });
                        }
                        // Numbered lists
                        else if let Some((num, item)) = trimmed.split_once(". ") {
                            if num.parse::<u32>().is_ok() {
                                in_list = true;
                                list_number += 1;
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("{}.", list_number))
                                            .size(14.0)
                                            .strong()
                                            .color(theme.primary_color()),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(item).size(15.0).color(theme.text_color()),
                                    );
                                });
                            }
                        }
                        // Blockquote
                        else if let Some(quote) = trimmed.strip_prefix("> ") {
                            Frame::NONE
                                .fill(Color32::from_rgb(40, 40, 45))
                                .corner_radius(CornerRadius::same(4))
                                .inner_margin(Margin::same(12))
                                .stroke(Stroke::new(2.0, theme.primary_color()))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(
                                        RichText::new(quote)
                                            .size(14.0)
                                            .color(theme.text_secondary_color())
                                            .italics(),
                                    );
                                });
                        }
                        // Regular paragraph
                        else {
                            in_list = false;
                            list_number = 0;

                            // Handle inline formatting
                            let formatted = self.apply_inline_formatting(trimmed, theme);
                            ui.label(formatted.size(15.0));
                        }
                    }
                }
                ContentPart::CodeBlock { language, code } => {
                    ui.add_space(8.0);
                    format_code_block(ui, code, language, theme.is_dark());
                    ui.add_space(8.0);
                }
                ContentPart::InlineCode(code) => {
                    Frame::NONE
                        .fill(Color32::from_rgb(55, 55, 60))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::symmetric(4, 2))
                        .show(ui, |ui| {
                            ui.monospace(
                                RichText::new(code)
                                    .size(13.0)
                                    .color(Color32::from_rgb(212, 165, 116)),
                            );
                        });
                }
            }
        }
    }

    fn apply_inline_formatting(&self, text: &str, theme: &super::super::Theme) -> RichText {
        // For now, just return the text without formatting
        RichText::new(text).color(theme.text_color())
    }

    fn render_tool_call_card(
        &self,
        ui: &mut Ui,
        tool_call: &ToolCall,
        theme: &super::super::Theme,
    ) {
        // Use the tool_calls module rendering
        // This is a simplified version inline here
        let (icon, title, border_color) = match tool_call.name.as_str() {
            "read_file" | "view" => ("📖", "View", Color32::from_rgb(100, 181, 246)),
            "write_file" | "edit" => ("📝", "Edit", Color32::from_rgb(76, 175, 80)),
            "create_file" => ("✨", "Create", Color32::from_rgb(156, 39, 176)),
            "bash" | "execute" => ("⚡", "Bash", Color32::from_rgb(255, 152, 0)),
            "search" => ("🔍", "Search", Color32::from_rgb(33, 150, 243)),
            "list_directory" => ("📁", "List", Color32::from_rgb(121, 85, 72)),
            _ => ("🔧", tool_call.name.as_str(), theme.border_color()),
        };

        Frame::NONE
            .fill(theme.surface_color())
            .corner_radius(CornerRadius::same(8))
            .stroke(Stroke::new(1.5, border_color))
            .show(ui, |ui| {
                ui.set_width(ui.available_width().min(600.0));

                // Header
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new(icon).size(16.0));
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(title)
                            .strong()
                            .size(13.0)
                            .color(theme.text_color()),
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        let status_icon = match tool_call.status {
                            ToolCallStatus::Pending => "⏳",
                            ToolCallStatus::Running => "🔄",
                            ToolCallStatus::Success => "✅",
                            ToolCallStatus::Error => "❌",
                        };
                        ui.label(RichText::new(status_icon).size(14.0));
                    });
                });

                ui.add_space(8.0);
            });
    }

    fn render_attachment(&self, ui: &mut Ui, attachment: &Attachment, theme: &super::super::Theme) {
        Frame::NONE
            .fill(Color32::from_rgb(50, 50, 55))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("📎").size(16.0));
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(&attachment.name)
                                .size(13.0)
                                .color(theme.text_color()),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} • {} bytes",
                                attachment.content_type, attachment.size
                            ))
                            .size(11.0)
                            .color(theme.muted_text_color()),
                        );
                    });
                });
            });
    }

    fn render_cursor_animation(&self, ui: &mut Ui) {
        // Blinking cursor effect
        let time = ui.ctx().input(|i| i.time);
        let blink = (time * 2.0).sin() > 0.0;

        if blink {
            ui.label(
                RichText::new("▋")
                    .size(16.0)
                    .color(Color32::from_rgb(212, 165, 116)),
            );
        } else {
            ui.label(RichText::new(" ").size(16.0));
        }
    }

    pub(super) fn render_loading_indicator(&self, ui: &mut Ui, theme: &super::super::Theme) {
        ui.horizontal(|ui| {
            ui.add_space(80.0);

            Frame::NONE
                .fill(theme.surface_color())
                .corner_radius(CornerRadius::same(16))
                .inner_margin(Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Animated dots
                        let time = ui.ctx().input(|i| i.time);
                        let dot_count = (time * 3.0) as usize % 4;
                        let dots = "●".repeat(dot_count + 1);

                        ui.label(RichText::new(dots).color(theme.primary_color()).size(12.0));

                        ui.add_space(12.0);

                        ui.label(
                            RichText::new("wgenty is thinking...")
                                .color(theme.muted_text_color())
                                .size(14.0),
                        );
                    });
                });
        });
    }
}
