//! Chat UI Component - Full recreation of Wgenty Code chat interface
//!
//! Features:
//! - Exact Claude.ai message styling
//! - Syntax highlighted code blocks with copy button
//! - Tool call visualization
//! - File attachments
//! - Thinking process expand/collapse
//! - Perfect markdown rendering

mod input;
mod message;

use super::chat_types::ChatMessage;
use egui::{
    CornerRadius, Frame, Margin, RichText, ScrollArea, Stroke, Ui, Vec2,
};

/// Chat panel - full recreation of Wgenty Code interface
pub struct ChatPanel {
    pub messages: Vec<ChatMessage>,
    pub input_text: String,
    pub is_loading: bool,
    pub scroll_to_bottom: bool,
    current_model: String,

    /// Callback for sending messages
    on_send_message: Option<Box<dyn Fn(Vec<crate::api::ChatMessage>) + Send>>,
}

impl Default for ChatPanel {
    fn default() -> Self {
        Self {
            messages: vec![
                ChatMessage::assistant("Hello! I'm wgenty, your AI coding companion. I can help you with:")
                    .with_thinking("The user has started a new conversation. I should greet them and explain my capabilities.")
            ],
            input_text: String::new(),
            is_loading: false,
            scroll_to_bottom: true,
            current_model: "claude-3-5-sonnet-20241022".to_string(),
            on_send_message: None,
        }
    }
}

impl ChatPanel {
    pub fn set_on_send_message<F>(&mut self, callback: F)
    where
        F: Fn(Vec<crate::api::ChatMessage>) + Send + 'static,
    {
        self.on_send_message = Some(Box::new(callback));
    }

    pub fn set_current_model(&mut self, model: impl Into<String>) {
        self.current_model = model.into();
    }

    /// Render the complete chat panel
    pub fn ui(&mut self, ui: &mut Ui, theme: &super::Theme) {
        let available_height = ui.available_height();
        let input_height = 120.0;
        let messages_height = available_height - input_height - 16.0;

        // Messages area with exact Claude styling
        Frame::NONE.fill(theme.background_darkest()).show(ui, |ui| {
            ui.set_min_height(messages_height);

            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(self.scroll_to_bottom)
                .show(ui, |ui| {
                    ui.add_space(24.0);

                    // Welcome banner for first load
                    if self.messages.len() <= 1 {
                        self.render_welcome_banner(ui, theme);
                    }

                    // Render all messages
                    let msg_count = self.messages.len();
                    // 先克隆所有消息，避免借用冲突
                    let mut messages_clone = self.messages.clone();
                    for (idx, message) in messages_clone.iter_mut().enumerate() {
                        self.render_message(ui, message, theme, idx == msg_count - 1);
                    }
                    // 更新展开状态
                    for (i, msg) in messages_clone.iter().enumerate() {
                        self.messages[i].thinking_expanded = msg.thinking_expanded;
                    }

                    // Loading indicator
                    if self.is_loading && !self.messages.iter().any(|m| m.is_streaming) {
                        self.render_loading_indicator(ui, theme);
                    }

                    ui.add_space(24.0);
                });
        });

        // Input area - fixed at bottom
        Frame::NONE.fill(theme.background_darkest()).show(ui, |ui| {
            self.render_input_area(ui, theme);
        });
    }

    fn render_welcome_banner(&self, ui: &mut Ui, theme: &super::Theme) {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);

            Frame::NONE
                .fill(theme.surface_color())
                .corner_radius(CornerRadius::same(20))
                .stroke(Stroke::new(1.0, theme.border_color()))
                .inner_margin(Margin::symmetric(24, 22))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().min(520.0));
                    ui.horizontal(|ui| {
                        super::brand_icon::show(ui, Vec2::new(112.0, 112.0));
                        ui.add_space(20.0);

                        ui.vertical(|ui| {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("wgenty")
                                    .size(32.0)
                                    .strong()
                                    .color(theme.text_color())
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(format!("Model: {}", self.current_model))
                                    .size(14.0)
                                    .color(theme.muted_text_color())
                            );
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("Code, files, search, and terminal operations in one workspace.")
                                    .size(14.0)
                                    .color(theme.text_color())
                            );
                        });
                    });
                });

            ui.add_space(24.0);

            let capabilities = vec![
                ("Code", "Write & edit code", "Generate and modify code in your project"),
                ("Files", "Read files", "Inspect source, configs, and generated output"),
                ("Search", "Search", "Locate symbols and patterns across the workspace"),
                ("Shell", "Run commands", "Execute terminal commands and inspect results"),
            ];

            for (tag, title, desc) in capabilities {
                Frame::NONE
                    .fill(theme.surface_color())
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width().min(460.0));
                        ui.horizontal(|ui| {
                            Frame::NONE
                                .fill(theme.background_darkest())
                                .corner_radius(CornerRadius::same(6))
                                .inner_margin(Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(tag)
                                            .size(11.0)
                                            .strong()
                                            .color(theme.primary_color())
                                    );
                                });
                            ui.add_space(12.0);

                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(title)
                                        .size(14.0)
                                        .strong()
                                        .color(theme.text_color())
                                );
                                ui.label(
                                    RichText::new(desc)
                                        .size(12.0)
                                        .color(theme.muted_text_color())
                                );
                            });
                        });
                    });

                ui.add_space(8.0);
            }

            ui.add_space(40.0);
        });
    }

    // Public API
    pub fn add_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
        self.scroll_to_bottom = true;
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.messages.push(ChatMessage::assistant(
            "Hello! I'm wgenty. How can I help you today?",
        ));
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.is_loading = loading;
    }

    pub fn update_last_message(&mut self, content: impl Into<String>) {
        if let Some(last) = self.messages.last_mut() {
            last.content = content.into();
        }
    }
}
