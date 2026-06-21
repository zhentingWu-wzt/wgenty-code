use ratatui::style::Color;

// Brand colors matching the original Wgenty Code aesthetic
pub const PRIMARY: Color = Color::Magenta;
pub const ACCENT: Color = Color::Rgb(255, 140, 66);
pub const DIM: Color = Color::Rgb(120, 120, 120);
pub const SUCCESS: Color = Color::Rgb(100, 255, 100);
pub const INFO: Color = Color::Rgb(137, 180, 250);
pub const ERROR: Color = Color::Rgb(255, 100, 100);
pub const WARNING: Color = Color::Rgb(255, 200, 100);

// Roles
pub const ROLE_USER: Color = Color::Rgb(100, 200, 255);
pub const ROLE_ASSISTANT: Color = Color::Rgb(200, 180, 255);
pub const ROLE_TOOL: Color = Color::Rgb(160, 160, 160);
pub const ROLE_SYSTEM: Color = Color::Rgb(180, 180, 140);

// Layout
pub const PROMPT_SYMBOL: &str = "▸";
