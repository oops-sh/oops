//! World-diff rendering.
//!
//! Two renderers over the same `Vec<Change>`:
//! - `render_porcelain`: the frozen machine interface — `A/M/D path` lines,
//!   byte-order sorted, deleted directories as a single `D path/` entry,
//!   never colored, empty output for no changes. Do not change it.
//! - `render_human`: the default — Created/Modified/Deleted sections with
//!   counts, optional ANSI color, one-line summary. No stability promise.
//!
//! Known limitation (spec'd): the line-oriented porcelain format cannot
//! represent paths containing newlines; `-z` is reserved for a future
//! NUL-terminated variant.

use crate::backend::{Change, ChangeKind};

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

/// The stable machine-readable format (`oops diff --porcelain`).
pub fn render_porcelain(changes: &[Change]) -> String {
    let mut out = String::new();
    for c in changes {
        out.push(c.kind.letter());
        out.push(' ');
        out.push_str(&rendered_path(c));
        out.push('\n');
    }
    out
}

/// The human-readable default: grouped sections, counts, summary.
pub fn render_human(changes: &[Change], color: bool) -> String {
    if changes.is_empty() {
        return "no changes\n".to_string();
    }

    let mut out = String::new();
    let sections = [
        (ChangeKind::Added, "Created", "created", GREEN),
        (ChangeKind::Modified, "Modified", "modified", YELLOW),
        (ChangeKind::Deleted, "Deleted", "deleted", RED),
    ];

    let mut summary = Vec::new();
    for (kind, heading, noun, tint) in sections {
        let items: Vec<&Change> = changes.iter().filter(|c| c.kind == kind).collect();
        if items.is_empty() {
            continue;
        }
        if color {
            out.push_str(tint);
        }
        out.push_str(&format!("{heading} ({})", items.len()));
        if color {
            out.push_str(RESET);
        }
        out.push('\n');
        for c in &items {
            out.push_str("  ");
            out.push_str(&rendered_path(c));
            out.push('\n');
        }
        out.push('\n');
        summary.push(format!("{} {noun}", items.len()));
    }
    out.push_str(&summary.join(", "));
    out.push('\n');
    out
}

fn rendered_path(c: &Change) -> String {
    let mut s = c.path.to_string_lossy().into_owned();
    if c.is_dir {
        s.push('/');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn change(kind: ChangeKind, path: &str, is_dir: bool) -> Change {
        Change {
            kind,
            path: PathBuf::from(path),
            is_dir,
        }
    }

    fn mixed() -> Vec<Change> {
        vec![
            change(ChangeKind::Deleted, "d", false),
            change(ChangeKind::Modified, "m", false),
            change(ChangeKind::Added, "n1", false),
            change(ChangeKind::Added, "n2", false),
            change(ChangeKind::Deleted, "sub", true),
        ]
    }

    #[test]
    fn porcelain_lines_and_dir_slash() {
        assert_eq!(render_porcelain(&mixed()), "D d\nM m\nA n1\nA n2\nD sub/\n");
    }

    #[test]
    fn porcelain_empty_is_empty() {
        assert_eq!(render_porcelain(&[]), "");
    }

    #[test]
    fn human_sections_and_summary() {
        assert_eq!(
            render_human(&mixed(), false),
            "Created (2)\n  n1\n  n2\n\n\
             Modified (1)\n  m\n\n\
             Deleted (2)\n  d\n  sub/\n\n\
             2 created, 1 modified, 2 deleted\n"
        );
    }

    #[test]
    fn human_omits_empty_sections_and_zero_counts() {
        let only_added = vec![change(ChangeKind::Added, "x", false)];
        assert_eq!(
            render_human(&only_added, false),
            "Created (1)\n  x\n\n1 created\n"
        );
    }

    #[test]
    fn human_empty_says_no_changes() {
        assert_eq!(render_human(&[], false), "no changes\n");
        assert_eq!(render_human(&[], true), "no changes\n");
    }

    #[test]
    fn human_color_wraps_headings_only() {
        let only_deleted = vec![change(ChangeKind::Deleted, "gone", false)];
        assert_eq!(
            render_human(&only_deleted, true),
            "\x1b[31mDeleted (1)\x1b[0m\n  gone\n\n1 deleted\n"
        );
    }

    #[test]
    fn human_no_color_has_no_escapes() {
        assert!(!render_human(&mixed(), false).contains('\x1b'));
    }
}
