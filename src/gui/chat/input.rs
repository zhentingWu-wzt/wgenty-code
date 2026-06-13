//! Chat input area — text editor, send button, message conversion.

use super::super::chat_types::{ChatMessage, MessageRole};
use super::ChatPanel;
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke, TextEdit, Ui, Vec2};

impl ChatPanel {
    pub(super) fn render_input_area(&mut self, ui: &mut Ui, theme: &super::super::Theme) {
        Frame::NONE
            .fill(theme.background_darkest())
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                // Input container
                Frame::NONE
                    .fill(theme.surface_color())
                    .corner_radius(CornerRadius::same(16))
                    .stroke(Stroke::new(1.0, theme.border_color()))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());

                        // Text input
                        let text_edit = TextEdit::multiline(&mut self.input_text)
                            .hint_text("Message wgenty... (Shift+Enter for new line)")
                            .desired_width(ui.available_width() - 60.0)
                            .min_size(Vec2::new(0.0, 48.0))
                            .margin(egui::vec2(8.0, 8.0))
                            .font(egui::TextStyle::Body);

                        let response = ui.add(text_edit);

                        // Send button
                        ui.add_space(8.0);

                        let button_enabled = !self.input_text.trim().is_empty() && !self.is_loading;
                        let button_color = if button_enabled {
                            theme.primary_color()
                        } else {
                            Color32::from_rgb(60, 60, 60)
                        };

                        let send_button = ui.add_sized(
                            Vec2::new(44.0, 44.0),
                            egui::Button::new(
                                RichText::new(if self.is_loading { "⏳" } else { "➤" })
                                    .size(20.0)
                                    .color(if button_enabled {
                                        Color32::WHITE
                                    } else {
                                        theme.muted_text_color()
                                    }),
                            )
                            .fill(button_color)
                            .corner_radius(CornerRadius::same(10)),
                        );

                        // Handle send
                        let enter_pressed = response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);

                        if (send_button.clicked() || enter_pressed) && button_enabled {
                            self.send_message();
                        }
                    });

                // Hint text
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Shift + Enter for new line • Enter to send")
                            .color(theme.muted_text_color())
                            .size(11.0),
                    );
                });
            });
    }

    fn send_message(&mut self) {
        let content = self.input_text.trim().to_string();
        if content.is_empty() {
            return;
        }

        // Add user message
        let user_msg = ChatMessage::user(&content);
        self.messages.push(user_msg);

        self.input_text.clear();
        self.is_loading = true;
        self.scroll_to_bottom = true;

        // Send via callback
        if let Some(callback) = &self.on_send_message {
            let api_messages = self.convert_to_api_messages();
            callback(api_messages);
        }
    }

    fn convert_to_api_messages(&self) -> Vec<crate::api::ChatMessage> {
        self.messages
            .iter()
            .filter_map(|msg| match msg.role {
                MessageRole::User => Some(crate::api::ChatMessage::user(&msg.content)),
                MessageRole::Assistant => Some(crate::api::ChatMessage::assistant(&msg.content)),
                MessageRole::System => None,
            })
            .collect()
    }
}
