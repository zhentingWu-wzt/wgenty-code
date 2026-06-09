//! HTML Templates - Template engine for web pages

mod index;
mod detail;
mod search;

pub struct TemplateEngine;

impl TemplateEngine {
    pub fn new() -> Self {
        Self
    }

    /// Render the main index page
    pub fn render_index(&self) -> String {
        index::render()
    }

    /// Render the plugin detail page
    pub fn render_plugin_detail(&self, plugin_id: &str) -> String {
        detail::render(plugin_id)
    }

    /// Render the search page
    pub fn render_search(&self) -> String {
        search::render()
    }
}

impl Default for TemplateEngine {
    fn default() -> Self {
        Self::new()
    }
}
