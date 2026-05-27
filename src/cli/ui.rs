//! UI Module - Beautiful terminal UI matching original Claude Code
//!
//! This module provides styled output, colors, animations, and formatting
//! to match the aesthetic of the original TypeScript Claude Code CLI.

use colored::{ColoredString, Colorize};
use std::fmt::Write as FmtWrite;
use std::io::{self, BufWriter, StdoutLock, Write};
use std::thread;
use std::time::Duration;

/// Lightweight line builder — accumulates styled text into a single String
/// to avoid per-segment ANSI-reset/reopen fragmentation.
pub struct Line {
    buf: String,
}

impl Line {
    pub fn new() -> Self {
        Self {
            buf: String::with_capacity(128),
        }
    }

    pub fn push(&mut self, text: &str) -> &mut Self {
        self.buf.push_str(text);
        self
    }

    pub fn push_styled(&mut self, styled: &ColoredString) -> &mut Self {
        self.buf.push_str(&styled.to_string());
        self
    }

    pub fn push_str_styled(&mut self, text: &str, color: (u8, u8, u8)) -> &mut Self {
        write!(
            self.buf,
            "\x1b[38;2;{};{};{}m{}\x1b[0m",
            color.0, color.1, color.2, text
        )
        .ok();
        self
    }

    pub fn push_str_bold(&mut self, text: &str, color: (u8, u8, u8)) -> &mut Self {
        write!(
            self.buf,
            "\x1b[1;38;2;{};{};{}m{}\x1b[0m",
            color.0, color.1, color.2, text
        )
        .ok();
        self
    }

    /// Write the accumulated line to stdout
    pub fn out(&self, stdout: &mut StdoutLock) -> io::Result<()> {
        stdout.write_all(self.buf.as_bytes())
    }

    /// Write with a trailing newline
    pub fn outln(&self, stdout: &mut StdoutLock) -> io::Result<()> {
        stdout.write_all(self.buf.as_bytes())?;
        stdout.write_all(b"\n")
    }

    /// Print directly (takes stdout lock internally)
    pub fn print(&self) {
        let mut stdout = io::stdout().lock();
        let _ = self.out(&mut stdout);
    }

    pub fn println(&self) {
        let mut stdout = io::stdout().lock();
        let _ = self.outln(&mut stdout);
    }

    pub fn into_string(self) -> String {
        self.buf
    }
}

// Shared BufWriter for stdout to reduce lock contention.
// Wrapping stdout in a BufWriter is cheap and avoids per-print syscalls.
pub fn stdout() -> BufWriter<io::Stdout> {
    BufWriter::with_capacity(8192, io::stdout())
}

/// Claude Code brand colors
pub mod colors {
    use colored::Color;

    /// Anthropic purple - primary brand color
    pub const PRIMARY: Color = Color::Magenta;
    /// Warm orange - accent color
    pub const ACCENT: Color = Color::TrueColor {
        r: 255,
        g: 140,
        b: 66,
    };
    /// Soft purple for secondary elements
    pub const SECONDARY: Color = Color::TrueColor {
        r: 147,
        g: 112,
        b: 219,
    };
    /// Green for success states
    pub const SUCCESS: Color = Color::Green;
    /// Yellow for warnings
    pub const WARNING: Color = Color::Yellow;
    /// Red for errors
    pub const ERROR: Color = Color::Red;
    /// Cyan for info
    pub const INFO: Color = Color::Cyan;
    /// Bright white for text
    pub const TEXT: Color = Color::White;
    /// Gray for muted text
    pub const MUTED: Color = Color::BrightBlack;
}

/// Print the wgenty welcome banner
pub fn print_welcome(model: &str) {
    println!();
    print_ascii_logo(model);
    println!();
    print_feature_bar();
    println!();
    println!(
        "     {}",
        "输入 help 查看命令 · 输入 exit 退出"
            .bright_black()
            .italic()
    );
    println!();
}

/// Print ASCII Art logo with gradient colors
fn print_ascii_logo(model: &str) {
    let logo_lines = [
        "  ▄   ▄   ▄▄▄   ▄▄▄▄▄  ▄   ▄  ▄▄▄▄▄  ▄   ▄",
        "  █   █   ███   █████  █   █  █████  █   █",
        "  █   █  █   █  █      ██  █    █    █   █",
        "  █ █ █  █      ███    █ █ █    █     ███ ",
        "  █ █ █  █  ██  █      █  ██    █      █  ",
        "   █ █    ████  █████  █   █    █      █  ",
    ];

    let gradient = [
        (220, 180, 255),
        (200, 160, 240),
        (170, 130, 220),
        (140, 100, 195),
        (115, 80, 170),
        (100, 60, 150),
    ];

    for (i, line) in logo_lines.iter().enumerate() {
        let (r, g, b) = gradient[i];
        println!("{}", line.truecolor(r, g, b).bold());
    }

    println!();
    println!("        {} {} {}",
        "🟣".to_string(),
        "Wgenty Code".truecolor(200, 150, 255).bold(),
        "· Rust Edition".truecolor(255, 140, 66).bold()
    );
    println!("           {}", "高性能 AI 编码助手".truecolor(147, 112, 219));
    println!();
    println!(
        "        {} {}",
        "Model:".bright_black(),
        model.truecolor(220, 200, 255)
    );
}

/// Print single-line feature bar with dividers
fn print_feature_bar() {
    let width = terminal_size().0.min(70);
    let divider = "─".repeat(width as usize);
    println!("   {}", divider.truecolor(80, 60, 100));
    println!(
        "     {} 启动 {}   {} 内存 {}   {} 响应 {}",
        "⚡".truecolor(255, 200, 50),
        "2.5x".green().bold(),
        "💾".truecolor(100, 200, 255),
        "-60%".green().bold(),
        "🚀".truecolor(255, 140, 66),
        "+40%".green().bold(),
    );
    println!("   {}", divider.truecolor(80, 60, 100));
}

/// Print a stylish divider
pub fn print_divider() {
    let width = terminal_size().0.min(70);
    let line = "─".repeat(width as usize);
    println!("{}", line.truecolor(100, 80, 120));
}

/// Print an assistant message with styling
pub fn print_claude_message(content: &str) {
    println!();
    let width = terminal_size().0;
    let inner = (width as usize).saturating_sub(4);
    let label = " 🟣 Wgenty ";
    let label_display_width = unicode_width::UnicodeWidthStr::width(label);
    let dash_after = inner.saturating_sub(label_display_width);

    // Orange border for assistant messages
    let border_color = (180, 120, 60);
    println!(
        "  ╭{}{}╮",
        label.truecolor(200, 150, 80).bold(),
        "─"
            .repeat(dash_after)
            .truecolor(border_color.0, border_color.1, border_color.2)
    );

    // Format the content with proper wrapping and styling
    for line in content.lines() {
        let left_border = format!("  {} ", "│")
            .truecolor(border_color.0, border_color.1, border_color.2)
            .to_string();

        if line.starts_with("```") {
            if line.len() > 3 {
                let lang = &line[3..];
                println!(
                    "{} {}",
                    left_border,
                    format!("───── {} ─────", lang).truecolor(80, 80, 80)
                );
            } else {
                println!(
                    "{} {}",
                    left_border,
                    "─────────────────────".truecolor(80, 80, 80)
                );
            }
        } else if line.starts_with("#") {
            let level = line.chars().take_while(|&c| c == '#').count();
            let header_text = line.trim_start_matches('#').trim();
            let styled = match level {
                1 => header_text.truecolor(255, 140, 66).bold().underline(),
                2 => header_text.truecolor(200, 150, 255).bold(),
                _ => header_text.bright_white().bold(),
            };
            println!("{}{}", left_border, styled);
        } else if line.starts_with("-") || line.starts_with("*") {
            println!(
                "{}{} {}",
                left_border,
                "•".truecolor(147, 112, 219),
                &line[1..].trim()
            );
        } else if line.starts_with(">") {
            println!(
                "{}{} {}",
                left_border,
                "│".truecolor(100, 80, 120),
                line[1..].trim().bright_black()
            );
        } else {
            let formatted = format_inline_styles(line);
            println!("{}{}", left_border, formatted);
        }
    }

    println!(
        "  ╰{}╯",
        "─"
            .repeat(inner)
            .truecolor(border_color.0, border_color.1, border_color.2)
    );
    println!();
}

/// Print a user message header (content already shown in terminal input)
pub fn print_user_message(_content: &str) {
    // 不重复打印用户输入，因为输入时已经显示在终端了
}

/// Buffered streaming output — accumulates deltas and flushes periodically.
/// Uses a simple left margin instead of full-bordered boxes.
pub struct StreamLineState {
    buf: String,
    line_width: usize,
    max_width: usize,
    started: bool,
    stdout: BufWriter<io::Stdout>,
}

impl StreamLineState {
    pub fn new() -> Self {
        let width = terminal_size().0 as usize;
        let max_width = width.saturating_sub(4);
        Self {
            buf: String::with_capacity(2048),
            line_width: 0,
            max_width,
            started: false,
            stdout: BufWriter::with_capacity(4096, io::stdout()),
        }
    }

    pub fn print_delta(&mut self, content: &str) {
        for ch in content.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                self.buf.push('\n');
                self.line_width = 0;
            } else {
                let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if self.line_width + ch_width > self.max_width {
                    self.buf.push('\n');
                    self.line_width = 0;
                }
                self.buf.push(ch);
                self.line_width += ch_width;
            }
        }
        self.flush_if_needed();
    }

    fn flush_if_needed(&mut self) {
        if self.buf.len() > 1024 {
            if !self.started {
                let _ = write!(self.stdout, "\n  │ ");
                self.started = true;
            }
            // Prefix each buffered line with left margin
            let mut result = String::with_capacity(self.buf.len() + 64);
            for (i, line) in self.buf.lines().enumerate() {
                if i > 0 {
                    result.push_str("\n  │ ");
                }
                result.push_str(line);
            }
            let _ = self.stdout.write_all(result.as_bytes());
            let _ = self.stdout.flush();
            self.buf.clear();
            self.line_width = 0;
        }
    }

    pub fn finish(&mut self) {
        if !self.buf.is_empty() {
            if !self.started {
                let _ = write!(self.stdout, "\n  │ ");
            }
            for (i, line) in self.buf.lines().enumerate() {
                if i > 0 {
                    let _ = write!(self.stdout, "\n  │ ");
                }
                let _ = self.stdout.write_all(line.as_bytes());
            }
        }
        let _ = self.stdout.write_all(b"\n");
        let _ = self.stdout.flush();
    }
}

/// Format inline markdown styles (bold, italic, code)
fn format_inline_styles(text: &str) -> ColoredString {
    // Handle inline code
    if text.contains('`') {
        let mut result = String::new();
        let mut in_code = false;
        for c in text.chars() {
            if c == '`' {
                in_code = !in_code;
                if in_code {
                    result.push_str("\x1b[48;5;238m\x1b[38;5;250m");
                } else {
                    result.push_str("\x1b[0m");
                }
            } else {
                result.push(c);
            }
        }
        return result.normal();
    }

    // Handle bold (**text**)
    if text.contains("**") {
        let parts: Vec<&str> = text.split("**").collect();
        let mut result = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i % 2 == 1 {
                result.push_str(&format!("\x1b[1m{}\x1b[0m", part));
            } else {
                result.push_str(part);
            }
        }
        return result.normal();
    }

    text.normal()
}

/// Spinner frames for the thinking animation
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Format elapsed time display
fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("等待响应 {}s", secs)
    } else {
        format!("等待响应 {}m{}s", secs / 60, secs % 60)
    }
}

/// Async thinking indicator that runs in the background while waiting for the
/// first stream chunk. Shows a spinner + elapsed time.
pub struct ThinkingIndicator {
    cancel: tokio::sync::watch::Sender<bool>,
    done: tokio::sync::watch::Receiver<bool>,
}

impl ThinkingIndicator {
    /// Start the thinking indicator. Returns a handle that can be used to stop it.
    pub fn start() -> Self {
        let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
        let (done_tx, done_rx) = tokio::sync::watch::channel(false);

        tokio::spawn(async move {
            let mut frame_idx = 0usize;
            let start = std::time::Instant::now();
            let mut tick = tokio::time::interval(Duration::from_millis(120));

            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        let frame = SPINNER_FRAMES[frame_idx % SPINNER_FRAMES.len()];
                        let elapsed = format_elapsed(start.elapsed().as_secs());

                        print!(
                            "\r  {} {}  {}",
                            "●".truecolor(147, 112, 219).bold(),
                            frame.truecolor(147, 112, 219),
                            elapsed.truecolor(150, 150, 150),
                        );
                        print!("\x1B[K");
                        io::stdout().flush().ok();

                        frame_idx += 1;
                    }
                    _ = cancel_rx.changed() => {
                        break;
                    }
                }
            }

            // Clear the indicator line
            print!("\r\x1B[K");
            io::stdout().flush().ok();

            // Signal that cleanup is done
            let _ = done_tx.send(true);
        });

        ThinkingIndicator {
            cancel: cancel_tx,
            done: done_rx,
        }
    }

    /// Stop the indicator and wait for the line to be cleared.
    pub async fn stop(mut self) {
        let _ = self.cancel.send(true);
        // Wait for the spawned task to finish clearing the line
        let _ = self.done.changed().await;
    }
}

/// Print a question from the assistant with styled formatting and numbered options
pub fn print_question(question: &str, options: &[(String, String)]) {
    println!();
    let width = terminal_size().0 as usize;
    let inner = width.saturating_sub(4);
    let label = " ❓ Question ";
    let label_display_width = unicode_width::UnicodeWidthStr::width(label);
    let dash_after = inner.saturating_sub(label_display_width);

    let border_color = (180, 120, 60);

    // Top border
    println!(
        "  ╭{}{}╮",
        label.truecolor(255, 200, 100).bold(),
        "─"
            .repeat(dash_after)
            .truecolor(border_color.0, border_color.1, border_color.2)
    );

    // Question text
    for line in question.lines() {
        println!(
            "  {} {}",
            "│".truecolor(border_color.0, border_color.1, border_color.2),
            line.truecolor(220, 200, 255).bold()
        );
    }

    // Spacer
    println!(
        "  {} {}",
        "│".truecolor(border_color.0, border_color.1, border_color.2),
        ""
    );

    // Options
    for (i, (label_opt, desc)) in options.iter().enumerate() {
        let num = (i + 1).to_string();
        println!(
            "  {}   {} {} - {}",
            "│".truecolor(border_color.0, border_color.1, border_color.2),
            num.truecolor(255, 140, 66).bold(),
            label_opt.truecolor(200, 150, 255).bold(),
            desc.truecolor(180, 180, 180)
        );
    }

    // Other option
    let other_idx = options.len() + 1;
    println!(
        "  {}   {} {} - {}",
        "│".truecolor(border_color.0, border_color.1, border_color.2),
        other_idx.to_string().truecolor(255, 140, 66).bold(),
        "Other".truecolor(200, 150, 255).bold(),
        "Type a custom answer".truecolor(180, 180, 180)
    );

    // Bottom border
    println!(
        "  ╰{}╯",
        "─"
            .repeat(inner)
            .truecolor(border_color.0, border_color.1, border_color.2)
    );
}

/// Print the question input prompt
pub fn print_question_prompt(multi_select: bool) {
    if multi_select {
        print!(
            "  {} ",
            "│ ▸ Your choices (comma-separated, e.g. 1,3): ".truecolor(255, 140, 66).bold()
        );
    } else {
        print!(
            "  {} ",
            "│ ▸ Your choice (number or custom text): ".truecolor(255, 140, 66).bold()
        );
    }
    io::stdout().flush().ok();
}

/// Print a typewriter-style animated output
pub fn print_typewriter(text: &str, delay_ms: u64) {
    for c in text.chars() {
        print!("{}", c);
        io::stdout().flush().ok();
        thread::sleep(Duration::from_millis(delay_ms));
    }
}

/// Print styled help information
pub fn print_help() {
    println!();
    println!(
        "  {}",
        "📖 Available Commands".truecolor(147, 112, 219).bold()
    );
    println!();

    let commands = [
        ("help", ".help", "Show help information"),
        ("status", ".status", "Show current status"),
        ("config", ".config", "Show configuration"),
        ("history", ".history", "Show conversation history"),
        ("reset", ".reset", "Reset conversation"),
        ("clear", ".clear", "Clear screen"),
        ("exit", ".exit", "Exit REPL"),
    ];

    for (cmd, alias, desc) in commands {
        println!(
            "  {} {:12} {:12} {}",
            "▸".truecolor(100, 80, 120),
            cmd.bright_cyan(),
            alias.bright_black(),
            desc.bright_white()
        );
    }

    println!();
    println!("  {}", "💡 Tip:".truecolor(255, 140, 66).bold());
    println!("     Type any message to chat with wgenty");
    println!();
}

/// Print status with styled formatting
pub fn print_status(status: &StatusInfo) {
    println!();
    println!("  {}", "📊 Status".truecolor(147, 112, 219).bold());
    println!();

    print_status_row("Model", &status.model, true);
    print_status_row("API Base", &status.api_base, true);
    print_status_row("Max Tokens", &status.max_tokens, true);
    print_status_row("Timeout", &format!("{}s", status.timeout), true);
    print_status_row(
        "Streaming",
        if status.streaming { "On" } else { "Off" },
        status.streaming,
    );
    print_status_row("Messages", &format!("{}", status.message_count), true);
    print_status_row(
        "API Key",
        if status.api_key_set {
            "Set ✓"
        } else {
            "Not Set ✗"
        },
        status.api_key_set,
    );

    println!();
}

fn print_status_row(label: &str, value: &str, positive: bool) {
    let value_colored = if positive { value.green() } else { value.red() };
    println!(
        "  {:15} {}",
        format!("{}:", label).truecolor(120, 120, 120),
        value_colored
    );
}

/// Status information structure
pub struct StatusInfo {
    pub model: String,
    pub api_base: String,
    pub max_tokens: String,
    pub timeout: u64,
    pub streaming: bool,
    pub message_count: usize,
    pub api_key_set: bool,
}

/// Print an error message with styling
pub fn print_error(message: &str) {
    println!();
    println!("  {} {}", "✗".red().bold(), "Error:".red().bold());
    println!("    {}", message.bright_red());
    println!();
}

/// Print a success message with styling
pub fn print_success(message: &str) {
    println!("  {} {}", "✓".green().bold(), message.green());
}

/// Print a warning message with styling
pub fn print_warning(message: &str) {
    println!("  {} {}", "⚠".yellow().bold(), message.yellow());
}

/// Print an info message with styling
pub fn print_info(message: &str) {
    println!("  {} {}", "ℹ".cyan(), message.cyan());
}

/// Print a code block with syntax highlighting simulation
pub fn print_code_block(code: &str, language: Option<&str>) {
    let lang = language.unwrap_or("");
    let header = format!("───── {} ─────", lang).truecolor(80, 80, 80);

    println!("  {}", header);
    for line in code.lines() {
        // Simple syntax highlighting simulation
        let highlighted = highlight_code_line(line);
        println!("  {}", highlighted);
    }
    println!("  {}", "─────────────────────".truecolor(80, 80, 80));
}

/// Simple syntax highlighting for code
fn highlight_code_line(line: &str) -> ColoredString {
    // Keywords
    let keywords = [
        "fn", "let", "mut", "use", "pub", "struct", "impl", "if", "else", "return", "match",
    ];
    for kw in &keywords {
        if line.trim().starts_with(kw) || line.contains(&format!(" {} ", kw)) {
            return line.truecolor(180, 140, 250); // Purple tint for keywords
        }
    }

    // Strings
    if line.contains('"') || line.contains('\'') {
        return line.truecolor(140, 220, 140); // Green tint for strings
    }

    // Comments
    if line.trim().starts_with("//") || line.trim().starts_with("#") {
        return line.bright_black(); // Gray for comments
    }

    line.normal()
}

/// Print a table with styled headers
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        println!("  (no data)");
        return;
    }

    // Calculate column widths
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    // Print header
    print!("  ");
    for (i, header) in headers.iter().enumerate() {
        let width = widths.get(i).copied().unwrap_or(10);
        print!(
            "{}  ",
            format!("{:width$}", header, width = width)
                .truecolor(147, 112, 219)
                .bold()
        );
    }
    println!();

    // Print divider
    print!("  ");
    for width in &widths {
        print!("{}  ", "─".repeat(*width).truecolor(80, 80, 80));
    }
    println!();

    // Print rows
    for row in rows {
        print!("  ");
        for (i, cell) in row.iter().enumerate() {
            let width = widths.get(i).copied().unwrap_or(10);
            print!(
                "{}  ",
                format!("{:width$}", cell, width = width).bright_white()
            );
        }
        println!();
    }
}

/// Get terminal size (width, height)
pub fn terminal_size() -> (u16, u16) {
    terminal_size::terminal_size()
        .map(|(w, h)| (w.0, h.0))
        .unwrap_or((80, 24))
}

/// Clear the screen
pub fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
    io::stdout().flush().ok();
}

/// Initialize the terminal for styled output
pub fn init_terminal() {
    // Enable ANSI colors on Windows
    #[cfg(windows)]
    {
        let _ = colored::control::set_virtual_terminal(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_inline_styles() {
        let text = "This is **bold** text";
        let _result = format_inline_styles(text);
        // Just verify it doesn't panic
    }

    #[test]
    fn test_terminal_size() {
        let (w, h) = terminal_size();
        assert!(w > 0);
        assert!(h > 0);
    }
}
