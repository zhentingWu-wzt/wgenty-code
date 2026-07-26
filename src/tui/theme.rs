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

// Inspector panel
pub const INSPECTOR_BORDER: Color = Color::Rgb(100, 100, 180);
pub const INSPECTOR_TAB_ACTIVE: Color = Color::Rgb(100, 200, 255);
pub const INSPECTOR_TAB_DIM: Color = Color::Rgb(80, 80, 80);
pub const INSPECTOR_SOURCE_BUILTIN: Color = Color::Rgb(60, 140, 220);
pub const INSPECTOR_SOURCE_FILE: Color = Color::Rgb(220, 180, 60);
pub const INSPECTOR_SOURCE_MEMORY: Color = Color::Rgb(100, 220, 120);
pub const INSPECTOR_SOURCE_SKILL: Color = Color::Rgb(100, 200, 200);
pub const INSPECTOR_SOURCE_CONFIG: Color = Color::Rgb(160, 160, 160);
pub const INSPECTOR_IMPORTANCE_HIGH: Color = Color::Rgb(220, 80, 80);
pub const INSPECTOR_IMPORTANCE_MID: Color = Color::Rgb(200, 160, 60);
pub const INSPECTOR_IMPORTANCE_LOW: Color = Color::Rgb(100, 100, 100);
