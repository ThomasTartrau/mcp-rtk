//! Side-by-side diff display for raw vs filtered tool responses.

use crate::display::*;

/// Display a colored diff between raw and filtered JSON strings.
///
/// Shows lines present only in raw (removed by filtering) in red,
/// lines present only in filtered in green, and common lines dimmed.
pub fn print_diff(raw: &str, filtered: &str, tool: &str, preset: Option<&str>) {
    let raw_pretty = prettify(raw);
    let filtered_pretty = prettify(filtered);

    let raw_lines: Vec<&str> = raw_pretty.lines().collect();
    let filtered_lines: Vec<&str> = filtered_pretty.lines().collect();

    // Header
    eprintln!();
    eprintln!("  {BOLD}{GREEN}mcp-rtk{RESET}{DIM} — diff{RESET}");
    eprintln!("  {DIM}{}{RESET}", "─".repeat(56));
    eprintln!();
    eprintln!("  {DIM}Tool:{RESET}    {BOLD}{tool}{RESET}");
    if let Some(p) = preset {
        eprintln!("  {DIM}Preset:{RESET}  {BOLD}{p}{RESET}");
    }

    let input_bytes = raw.len();
    let output_bytes = filtered.len();
    let saved = input_bytes.saturating_sub(output_bytes);
    let pct = if input_bytes > 0 {
        (saved as f64 / input_bytes as f64) * 100.0
    } else {
        0.0
    };
    let pct_color = pct_to_color(pct);

    eprintln!(
        "  {DIM}Input:{RESET}   {} bytes (~{} tokens)",
        input_bytes,
        input_bytes / 4
    );
    eprintln!(
        "  {DIM}Output:{RESET}  {} bytes (~{} tokens)",
        output_bytes,
        output_bytes / 4
    );
    eprintln!("  {DIM}Saved:{RESET}   {pct_color}{BOLD}{saved} bytes ({pct:.1}%){RESET}");
    eprintln!();
    eprintln!("  {RED}--- raw{RESET}    {GREEN}+++ filtered{RESET}");
    eprintln!("  {DIM}{}{RESET}", "─".repeat(56));

    // Simple line-based diff using longest common subsequence
    let ops = compute_diff(&raw_lines, &filtered_lines);
    for op in &ops {
        match op {
            DiffOp::Equal(line) => {
                println!("  {DIM} {line}{RESET}");
            }
            DiffOp::Remove(line) => {
                println!("  {RED}-{line}{RESET}");
            }
            DiffOp::Add(line) => {
                println!("  {GREEN}+{line}{RESET}");
            }
        }
    }
    eprintln!();
}

fn prettify(s: &str) -> String {
    serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| s.to_string())
}

enum DiffOp<'a> {
    Equal(&'a str),
    Remove(&'a str),
    Add(&'a str),
}

const MAX_DIFF_LINES: usize = 10_000;

fn compute_diff<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<DiffOp<'a>> {
    let m = old.len().min(MAX_DIFF_LINES);
    let n = new.len().min(MAX_DIFF_LINES);
    let old = &old[..m];
    let new = &new[..n];

    // Build LCS table
    let mut table = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if old[i - 1] == new[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = table[i - 1][j].max(table[i][j - 1]);
            }
        }
    }

    // Backtrack to produce diff
    let mut ops = Vec::new();
    let mut i = m;
    let mut j = n;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            ops.push(DiffOp::Equal(old[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            ops.push(DiffOp::Add(new[j - 1]));
            j -= 1;
        } else {
            ops.push(DiffOp::Remove(old[i - 1]));
            i -= 1;
        }
    }

    ops.reverse();
    ops
}
