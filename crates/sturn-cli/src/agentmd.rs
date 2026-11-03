//! The managed `AGENTS.md` agent-awareness block — `init` writes
//! it so every sturn-wired repo carries the same operating rules.
//!
//! The block is marker-delimited so human prose elsewhere in the
//! file survives syncs; a stale or hand-written `Sturn awareness`
//! section is upgraded in place, everything else is untouched.

use std::path::Path;

/// Marker pair delimiting the sturn-managed block.
pub const BEGIN: &str = "<!-- sturn:agent-awareness -->";
/// Closing marker.
pub const END: &str = "<!-- /sturn:agent-awareness -->";

/// Canonical managed section, between the markers — the text
/// deployed org-wide in sturn-wired repos.
const SECTION: &str = "\
## Sturn awareness

- **No PRs.** The pushed branch/commit is the deliverable; review
  streams continuously (sturn WAL), merge is a human/sturn call.
- sturn-wired repo (`sturn.toml` present): `sturn claim \"<uri>\"
  --ws <ws>` *before* edits — claims surface conflicts in ms.
- Ops via `sturn write|rename|delete`; `sturn snap` folds the ws
  into a real commit; `sturn push` publishes heads + refs/sturn/*.
- `sturn leave` on exit — drops the actor's claims.
";

/// The canonical emitted block, markers included.
fn canonical() -> String {
    format!("{BEGIN}\n\n{SECTION}\n{END}\n")
}

/// What a `sync` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// `AGENTS.md` was created.
    Created,
    /// The block was inserted or rewritten in an existing file.
    Updated,
    /// Already canonical — nothing written.
    Unchanged,
}

/// ATX heading level for a line (`## foo` → 2), else 0.
fn heading_level(line: &str) -> usize {
    let t = line.trim_start();
    let n = t.bytes().take_while(|b| *b == b'#').count();
    if n > 0 && t[n..].starts_with(char::is_whitespace) {
        n
    } else {
        0
    }
}

/// Heading text for a line, if it's an ATX heading.
fn heading_text(line: &str) -> Option<&str> {
    let n = heading_level(line);
    (n > 0).then(|| line.trim_start()[n..].trim())
}

/// Index of the `Sturn awareness` heading line (any level).
fn section_start(lines: &[String]) -> Option<usize> {
    lines
        .iter()
        .position(|l| heading_text(l).is_some_and(|t| t.eq_ignore_ascii_case("sturn awareness")))
}

/// Merge the canonical block into `src`:
/// - a marked block is rewritten in place;
/// - an unmarked `Sturn awareness` section is replaced (upgrade);
/// - stray markers are dropped before either rule runs;
/// - otherwise the block is appended.
///
/// Emission is deterministic: exactly one blank line on either
/// side of the block, file ends with a single newline.
fn merged(src: &str) -> String {
    let canon: Vec<String> = canonical()
        .trim_end_matches('\n')
        .split('\n')
        .map(str::to_string)
        .collect();
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();

    let begin = lines.iter().position(|l| l.trim() == BEGIN);
    let end = begin.and_then(|b| {
        lines
            .iter()
            .enumerate()
            .skip(b + 1)
            .find(|(_, l)| l.trim() == END)
            .map(|(i, _)| i)
    });

    // [..h) head prose, [h..e) replaced span, [e..) tail prose
    let (h, e) = match (begin, end) {
        (Some(b), Some(e)) => (b, e + 1),
        _ => {
            // malformed or absent markers: drop strays, then try a
            // hand-written section
            lines.retain(|l| {
                let t = l.trim();
                t != BEGIN && t != END
            });
            match section_start(&lines) {
                Some(s) => {
                    let lvl = heading_level(&lines[s]);
                    let e = lines[s + 1..]
                        .iter()
                        .position(|l| {
                            let n = heading_level(l);
                            n > 0 && n <= lvl
                        })
                        .map_or(lines.len(), |p| s + 1 + p);
                    (s, e)
                }
                None => (lines.len(), lines.len()),
            }
        }
    };

    let mut out: Vec<String> = lines[..h].to_vec();
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    if !out.is_empty() {
        out.push(String::new());
    }
    out.extend(canon);
    let tail = lines[e..]
        .iter()
        .skip_while(|l| l.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if !tail.is_empty() {
        out.push(String::new());
        out.extend(tail);
    }
    // Canonical emission ends with exactly one newline — trailing
    // blank lines are dropped so a repeat `sync` is a no-op.
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    format!("{}\n", out.join("\n"))
}

/// Write the canonical block into `<root>/AGENTS.md`.
pub fn sync(root: &Path) -> Result<Change, String> {
    let path = root.join("AGENTS.md");
    let existed = path.exists();
    let src = std::fs::read_to_string(&path).unwrap_or_default();
    let next = merged(&src);
    if existed && next == src {
        return Ok(Change::Unchanged);
    }
    std::fs::write(&path, &next).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(if existed {
        Change::Updated
    } else {
        Change::Created
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sturn-agentmd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_file_created_then_unchanged() {
        let d = scratch("missing");
        assert_eq!(sync(&d).unwrap(), Change::Created);
        assert_eq!(
            std::fs::read_to_string(d.join("AGENTS.md")).unwrap(),
            canonical()
        );
        assert_eq!(sync(&d).unwrap(), Change::Unchanged);
    }

    #[test]
    fn appends_to_existing_prose() {
        let d = scratch("append");
        std::fs::write(d.join("AGENTS.md"), "# Repo\n\nRules here.\n").unwrap();
        assert_eq!(sync(&d).unwrap(), Change::Updated);
        let out = std::fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert!(out.starts_with("# Repo\n\nRules here.\n\n"));
        assert!(out.contains(BEGIN));
        assert!(out.ends_with(&format!("{END}\n")));
        assert_eq!(sync(&d).unwrap(), Change::Unchanged);
    }

    #[test]
    fn rewrites_drifted_marked_block() {
        let d = scratch("drift");
        let bad = canonical().replace("sturn leave", "sturn yeet");
        std::fs::write(d.join("AGENTS.md"), &bad).unwrap();
        assert_eq!(sync(&d).unwrap(), Change::Updated);
        assert_eq!(
            std::fs::read_to_string(d.join("AGENTS.md")).unwrap(),
            canonical()
        );
    }

    #[test]
    fn keeps_tail_prose_after_marked_block() {
        let d = scratch("tail");
        std::fs::write(
            d.join("AGENTS.md"),
            format!("# R\n\n{BEGIN}\n\nold\n\n{END}\n\n\n## Layout\n\nstuff\n\n\n"),
        )
        .unwrap();
        sync(&d).unwrap();
        let out = std::fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert!(out.starts_with("# R\n"));
        assert!(out.ends_with("## Layout\n\nstuff\n"));
        assert!(!out.contains("\nold\n"));
        assert_eq!(sync(&d).unwrap(), Change::Unchanged);
    }

    #[test]
    fn upgrades_handwritten_section() {
        let d = scratch("handwritten");
        std::fs::write(
            d.join("AGENTS.md"),
            "# Repo\n\n## Sturn awareness\n\n- custom bullet\n\n## Layout\n\nstuff\n",
        )
        .unwrap();
        sync(&d).unwrap();
        let out = std::fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert!(!out.contains("custom bullet"));
        assert!(out.contains("## Layout"));
        assert_eq!(sync(&d).unwrap(), Change::Unchanged);
    }

    #[test]
    fn handwritten_section_running_to_eof() {
        let d = scratch("handwritten-eof");
        std::fs::write(
            d.join("AGENTS.md"),
            "# Repo\n\n## Sturn awareness\n\n- custom bullet\n",
        )
        .unwrap();
        sync(&d).unwrap();
        let out = std::fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert!(!out.contains("custom bullet"));
        assert!(out.contains(BEGIN));
    }

    #[test]
    fn drops_stray_markers() {
        let d = scratch("stray");
        std::fs::write(d.join("AGENTS.md"), format!("# R\n\n{BEGIN}\n\nstuff\n")).unwrap();
        sync(&d).unwrap();
        let out = std::fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert_eq!(out.matches(BEGIN).count(), 1);
        assert_eq!(sync(&d).unwrap(), Change::Unchanged);
    }

    #[test]
    fn normalizes_trailing_blank_lines() {
        let d = scratch("trailblanks");
        std::fs::write(d.join("AGENTS.md"), "# Repo\n\n\n").unwrap();
        sync(&d).unwrap();
        let out = std::fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert!(out.starts_with("# Repo\n\n"));
        assert!(out.ends_with(&format!("{END}\n")));
        assert!(!out.ends_with("\n\n"));
        assert_eq!(sync(&d).unwrap(), Change::Unchanged);
    }

    #[test]
    fn write_error_surfaces() {
        let d = scratch("isdir");
        std::fs::create_dir_all(d.join("AGENTS.md")).unwrap();
        assert!(sync(&d).is_err());
    }
}
