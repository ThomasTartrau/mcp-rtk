//! Shared ANSI display utilities used by `tracking` and `discover`.

// ANSI color codes
pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const CYAN: &str = "\x1b[36m";
pub const WHITE: &str = "\x1b[37m";
pub const RED: &str = "\x1b[31m";

// Block characters for bars
const BLOCK_FULL: &str = "█";
const BLOCK_7: &str = "▉";
const BLOCK_6: &str = "▊";
const BLOCK_5: &str = "▋";
const BLOCK_4: &str = "▌";
const BLOCK_3: &str = "▍";
const BLOCK_2: &str = "▎";
const BLOCK_1: &str = "▏";

/// Render a block bar using Unicode fractional block characters.
///
/// `ratio` is 0.0–1.0, `width` is the total character width.
pub fn render_block_bar(ratio: f64, width: usize) -> String {
    let blocks = [
        " ", BLOCK_1, BLOCK_2, BLOCK_3, BLOCK_4, BLOCK_5, BLOCK_6, BLOCK_7, BLOCK_FULL,
    ];
    let total_eighths = (ratio.clamp(0.0, 1.0) * width as f64 * 8.0).round() as usize;
    let full_blocks = total_eighths / 8;
    let remainder = total_eighths % 8;
    let empty = width
        .saturating_sub(full_blocks)
        .saturating_sub(if remainder > 0 { 1 } else { 0 });

    let color = if ratio >= 0.7 {
        GREEN
    } else if ratio >= 0.3 {
        YELLOW
    } else {
        WHITE
    };

    format!(
        "{color}{}{}{DIM}{}{RESET}",
        BLOCK_FULL.repeat(full_blocks),
        if remainder > 0 { blocks[remainder] } else { "" },
        "░".repeat(empty),
    )
}

/// Pick a color based on percentage threshold.
pub fn pct_to_color(pct: f64) -> &'static str {
    if pct >= 70.0 {
        GREEN
    } else if pct >= 30.0 {
        YELLOW
    } else {
        WHITE
    }
}

/// Convert bytes to estimated tokens (bytes / 4, ceiling) and format with K/M suffix.
pub fn format_tokens(bytes: i64) -> String {
    format_number((bytes + 3) / 4)
}

/// Format a number with K/M suffixes (e.g. `"1.5K"`, `"2.3M"`).
pub fn format_number(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}

/// Truncate a display name to `max` characters, appending `"..."` if needed.
///
/// UTF-8 safe: the cut point is adjusted to a valid character boundary.
pub fn truncate_name(name: &str, max: usize) -> String {
    if name.len() > max && max >= 4 {
        // Find a valid char boundary at or before max - 3
        let mut end = max - 3;
        while end > 0 && !name.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &name[..end])
    } else if name.len() > max {
        let mut end = max;
        while end > 0 && !name.is_char_boundary(end) {
            end -= 1;
        }
        name[..end].to_string()
    } else {
        name.to_string()
    }
}
