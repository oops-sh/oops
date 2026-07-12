//! World-diff rendering. Phase 0: plain text, one `A/M/D path` line each,
//! sorted by path, directories with a trailing `/`.

use crate::backend::Change;

pub fn render(changes: &[Change]) -> String {
    let mut out = String::new();
    for c in changes {
        out.push(c.kind.letter());
        out.push(' ');
        out.push_str(&c.path.to_string_lossy());
        if c.is_dir {
            out.push('/');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ChangeKind;
    use std::path::PathBuf;

    #[test]
    fn renders_letters_and_dir_slash() {
        let changes = vec![
            Change {
                kind: ChangeKind::Added,
                path: PathBuf::from("n"),
                is_dir: false,
            },
            Change {
                kind: ChangeKind::Deleted,
                path: PathBuf::from("sub"),
                is_dir: true,
            },
            Change {
                kind: ChangeKind::Modified,
                path: PathBuf::from("m"),
                is_dir: false,
            },
        ];
        assert_eq!(render(&changes), "A n\nD sub/\nM m\n");
    }
}
