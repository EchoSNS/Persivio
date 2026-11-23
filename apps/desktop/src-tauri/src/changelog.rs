//! Release notes from `CHANGELOG.md`, built into the app so "What's new" works offline.

use serde::{Deserialize, Serialize};
use specta::Type;

/// The repository's changelog, embedded at compile time.
const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");

/// One release's notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ReleaseNotesView {
    /// Version, e.g. `0.2.0`.
    pub version: String,
    /// Release date as written (`YYYY-MM-DD`).
    pub date: String,
    /// Sections in order (`Added`, `Fixed`, …), each with its items.
    pub sections: Vec<NotesSection>,
}

/// A titled list of changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct NotesSection {
    /// `Added`, `Changed`, `Fixed`, `Security`, …
    pub title: String,
    /// One entry per change (plain text; markdown markers removed).
    pub items: Vec<String>,
}

/// Parses `## [x.y.z] - date` releases with `### Section` lists. Continuation lines
/// (indented) are joined to the previous item. Newest first, as written.
#[must_use]
pub fn parse(markdown: &str) -> Vec<ReleaseNotesView> {
    let mut releases: Vec<ReleaseNotesView> = Vec::new();
    for line in markdown.lines() {
        if let Some(heading) = line.strip_prefix("## [") {
            let (version, rest) = heading.split_once(']').unwrap_or((heading, ""));
            releases.push(ReleaseNotesView {
                version: version.trim().to_owned(),
                date: rest.trim_start_matches([' ', '-']).trim().to_owned(),
                sections: Vec::new(),
            });
            continue;
        }
        let Some(release) = releases.last_mut() else {
            continue;
        };
        if let Some(title) = line.strip_prefix("### ") {
            release.sections.push(NotesSection {
                title: title.trim().to_owned(),
                items: Vec::new(),
            });
        } else if let Some(section) = release.sections.last_mut() {
            let trimmed = line.trim();
            if let Some(item) = trimmed.strip_prefix("- ") {
                section.items.push(item.replace('`', ""));
            } else if !trimmed.is_empty()
                && line.starts_with(' ')
                && let Some(last) = section.items.last_mut()
            {
                last.push(' ');
                last.push_str(&trimmed.replace('`', ""));
            }
        }
    }
    releases
}

/// Every release in the embedded changelog.
#[must_use]
pub fn all() -> Vec<ReleaseNotesView> {
    parse(CHANGELOG)
}

/// `major.minor.patch` as numbers (missing parts are 0; pre-release tags ignored).
#[must_use]
pub fn version_key(version: &str) -> (u64, u64, u64) {
    let core = version.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.').map(|p| p.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_sections_and_wrapped_items_are_parsed() {
        let md = "# Changelog\n\nIntro\n\n## [1.1.0] - 2026-12-01\n\n### Added\n- New `thing`\n  continued here\n- Second\n\n### Fixed\n- A bug\n\n## [1.0.0] - 2026-11-01\n### Added\n- First\n";
        let r = parse(md);
        assert_eq!(r.len(), 2);
        assert_eq!(
            (r[0].version.as_str(), r[0].date.as_str()),
            ("1.1.0", "2026-12-01")
        );
        assert_eq!(r[0].sections[0].title, "Added");
        assert_eq!(
            r[0].sections[0].items,
            ["New thing continued here", "Second"]
        );
        assert_eq!(r[0].sections[1].items, ["A bug"]);
        assert_eq!(r[1].sections[0].items, ["First"]);
    }

    #[test]
    fn the_embedded_changelog_has_this_version_first() {
        let releases = all();
        assert_eq!(
            releases.first().map(|r| r.version.as_str()),
            Some(env!("CARGO_PKG_VERSION")),
            "add a CHANGELOG.md entry for the new version"
        );
        assert!(releases.iter().all(|r| !r.sections.is_empty()));
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(version_key("0.10.0") > version_key("0.9.9"));
        assert_eq!(version_key("1.2.3-beta.1"), (1, 2, 3));
        assert_eq!(version_key("2"), (2, 0, 0));
    }
}
