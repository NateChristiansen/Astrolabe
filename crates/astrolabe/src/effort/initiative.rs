//! An initiative page as `astrolabe initiative new` reads and writes it: the body is the
//! page, and its `## Efforts` section names the efforts to file, in order.

use super::file::fence_mask;

/// The body of an `initiative new` command, taken apart: what comes before the efforts
/// section, the section's heading as written, each effort as title and goal, and everything
/// from the next section on, kept verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitiativePage {
    pub before: String,
    pub heading: String,
    pub efforts: Vec<(String, String)>,
    pub joins: Vec<u32>,
    pub after: String,
}

impl InitiativePage {
    /// Reads the body. The efforts section is the first unfenced `## ` heading that starts
    /// with "Efforts" and runs to the next `## ` heading; in it, each paragraph is one
    /// effort, the title on its first line and the goal below. A leading `# ` title line is
    /// dropped, because the page gets the command's title.
    pub fn parse(body: &str) -> Result<Self, String> {
        let body = body.replace("\r\n", "\n");
        let lines: Vec<&str> = body.lines().collect();
        let fenced = fence_mask(lines.iter().copied());
        let heading_at = |i: usize| -> Option<&str> {
            (!fenced[i])
                .then(|| lines[i].strip_prefix("## "))
                .flatten()
                .map(str::trim)
        };
        let Some(start) = (0..lines.len())
            .find(|&i| heading_at(i).is_some_and(|h| h.to_lowercase().starts_with("efforts")))
        else {
            let mut before = lines.clone();
            if before.first().is_some_and(|l| l.starts_with("# ")) {
                before.remove(0);
            }
            return Ok(Self {
                before: before.join("\n").trim().to_owned(),
                ..Default::default()
            });
        };
        let end = (start + 1..lines.len())
            .find(|&i| heading_at(i).is_some())
            .unwrap_or(lines.len());
        let mut before: Vec<&str> = lines[..start].to_vec();
        if before
            .iter()
            .find(|l| !l.trim().is_empty())
            .is_some_and(|l| l.starts_with("# "))
        {
            let at = before.iter().position(|l| l.starts_with("# ")).unwrap_or(0);
            before.remove(at);
        }
        let mut efforts = Vec::new();
        let mut joins = Vec::new();
        // A blank line is one with nothing but whitespace, as Markdown reads it, so a line
        // of spaces between two efforts still separates them (review, 2026-11-07).
        let mut paragraphs: Vec<Vec<&str>> = vec![Vec::new()];
        for line in &lines[start + 1..end] {
            if line.trim().is_empty() {
                paragraphs.push(Vec::new());
            } else if let Some(last) = paragraphs.last_mut() {
                last.push(line);
            }
        }
        for paragraph in paragraphs {
            let mut parts = paragraph.into_iter().map(str::trim);
            let Some(title) = parts.next().filter(|t| !t.is_empty()) else {
                continue;
            };
            if title == "effort" || title.starts_with("effort ") {
                let digits: String = title
                    .strip_prefix("effort ")
                    .unwrap_or("")
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                let number = digits.parse::<u32>().ok().filter(|n| *n > 0).ok_or(
                    "a join paragraph begins exactly `effort <n>`, with an existing effort number",
                )?;
                if joins.contains(&number) {
                    return Err(format!("duplicate effort {number} in join"));
                }
                joins.push(number);
                continue;
            }
            let title = ["- ", "* "]
                .iter()
                .find_map(|m| title.strip_prefix(m))
                .or_else(|| {
                    title
                        .split_once(". ")
                        .filter(|(n, _)| n.parse::<u32>().is_ok())
                        .map(|(_, t)| t)
                })
                .unwrap_or(title)
                .trim()
                .to_owned();
            let goal = parts.collect::<Vec<_>>().join("\n").trim().to_owned();
            if goal.is_empty() {
                return Err(format!(
                    "effort {title:?} has no goal: write it on the lines below its title"
                ));
            }
            efforts.push((title, goal));
        }
        if !efforts.is_empty() && !joins.is_empty() {
            return Err("an Efforts section either creates efforts or joins existing numbers; do not mix them".into());
        }
        if efforts.is_empty() && joins.is_empty() {
            return Err("the `## Efforts` section is empty: one paragraph per effort, the title on its first line, the goal below it".to_owned());
        }
        Ok(Self {
            before: before.join("\n").trim().to_owned(),
            heading: heading_at(start).unwrap_or("Efforts").to_owned(),
            efforts,
            joins,
            after: lines[end..].join("\n").trim().to_owned(),
        })
    }

    /// The page as written to `initiatives/`: the properties the board reads, the title, the
    /// purpose, the efforts as a numbered list of links in their filed order, then the rest
    /// of the body as given. `filed` pairs each effort's note path (from the notes root) with
    /// its number, in the order of `efforts`.
    pub fn render(&self, title: &str, repo: &str, day: &str, filed: &[(u32, String)]) -> String {
        let mut out = format!("---\nstatus: ready\nrepo: {repo}\ncreated: {day}\n---\n# {title}\n");
        if !self.before.is_empty() {
            out += &format!("\n{}\n", self.before);
        }
        if !self.heading.is_empty() {
            out += &format!("\n## {}\n\n", self.heading);
        }
        for (i, ((name, goal), (_, rel))) in self.efforts.iter().zip(filed).enumerate() {
            let goal = goal.split_whitespace().collect::<Vec<_>>().join(" ");
            if goal.is_empty() {
                out += &format!("{}. [{name}](../{rel})\n", i + 1);
            } else {
                out += &format!("{}. [{name}](../{rel}). {goal}\n", i + 1);
            }
        }
        if !self.after.is_empty() {
            out += &format!("\n{}\n", self.after);
        }
        let mut file = super::EffortFile::parse(&out).expect("validated initiative body");
        file.set("waits_for", "");
        file.render_initiative()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_the_page_and_its_efforts_section_names_the_efforts() {
        let body = "# Ink\n\nSmooth the ink.\n\n```\n## Efforts\nnot this one\n```\n\n## Efforts and order\n1. Tether cap\nCap the tether.\n  \n- Goo at speed\nKeep goo stable\nat speed.\n\n## What completion means\n- never splits\n";
        let page = InitiativePage::parse(body).unwrap();
        assert_eq!(
            page.before,
            "Smooth the ink.\n\n```\n## Efforts\nnot this one\n```"
        );
        assert_eq!(page.heading, "Efforts and order");
        assert_eq!(
            page.efforts,
            vec![
                ("Tether cap".to_owned(), "Cap the tether.".to_owned()),
                (
                    "Goo at speed".to_owned(),
                    "Keep goo stable\nat speed.".to_owned()
                ),
            ]
        );
        assert_eq!(page.after, "## What completion means\n- never splits");
        let filed = vec![
            (2, "efforts/2026/11-November/0002-tether-cap.md".to_owned()),
            (
                3,
                "efforts/2026/11-November/0003-goo-at-speed.md".to_owned(),
            ),
        ];
        let text = page.render("Ink", "demo", "2026-11-07", &filed);
        assert!(text.contains("## Purpose\n\nSmooth the ink."), "{text}");
        assert!(text.contains("## Decisions\n"));
        assert!(text.contains(
            "1. [Tether cap](../efforts/2026/11-November/0002-tether-cap.md). Cap the tether."
        ));
        assert_eq!(
            InitiativePage::parse("Purpose only.").unwrap().before,
            "Purpose only."
        );
        for (body, why) in [
            ("## Efforts\n\n## Done\n", "is empty"),
            ("## Efforts\nTitle only\n", "has no goal"),
        ] {
            let err = InitiativePage::parse(body).unwrap_err();
            assert!(err.contains(why), "{body:?}: {err}");
        }
    }
}

/// One page in the shared, unambiguous initiative namespace.
#[derive(Debug, Clone)]
pub struct Initiative {
    pub slug: String,
    pub path: String,
    pub file: super::EffortFile,
    pub text: String,
}

/// Both titles and slugs accept legacy wikilinks; filenames remain canonical.
pub fn initiative_name(name: &str) -> String {
    let name = name.trim();
    name.strip_prefix("[[")
        .and_then(|n| n.strip_suffix("]]"))
        .unwrap_or(name)
        .trim()
        .to_lowercase()
}

pub fn initiative_catalog(root: &std::path::Path) -> Result<Vec<Initiative>, String> {
    let dir = root.join("initiatives");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut pages = Vec::new();
    let mut aliases = std::collections::HashMap::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let slug = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("initiative filename isn't text")?
            .to_owned();
        let rel = format!("initiatives/{}.md", slug);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
        let file = super::EffortFile::parse(&text).map_err(|e| format!("{rel}: {e}"))?;
        if file.title.is_empty() {
            return Err(format!("{rel}: initiative has no title"));
        }
        for alias in [initiative_name(&slug), initiative_name(&file.title)] {
            if let Some(other) = aliases.insert(alias.clone(), rel.clone())
                && other != rel
            {
                return Err(format!("ambiguous initiative {alias:?}: {other} and {rel}"));
            }
        }
        pages.push(Initiative {
            slug,
            path: rel,
            file,
            text,
        });
    }
    pages.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(pages)
}

pub fn resolve_initiative<'a>(
    pages: &'a [Initiative],
    name: &str,
) -> Result<&'a Initiative, String> {
    let name = initiative_name(name);
    let matches: Vec<_> = pages
        .iter()
        .filter(|p| initiative_name(&p.slug) == name || initiative_name(&p.file.title) == name)
        .collect();
    match matches.as_slice() {
        [page] => Ok(page),
        [] => Err(format!(
            "no initiative page for {name:?}; file it with `astrolabe initiative new`"
        )),
        _ => Err(format!("ambiguous initiative {name:?}")),
    }
}

/// Membership comes only from frontmatter, across every repository. Unreadable notes
/// fail closed so an initiative cannot be accepted with an unexamined member.
pub fn initiative_members(
    root: &std::path::Path,
    page: &Initiative,
) -> Result<Vec<(String, super::EffortFile)>, String> {
    fn walk(
        root: &std::path::Path,
        rel: &str,
        depth: u8,
        files: &mut Vec<String>,
    ) -> Result<(), String> {
        let entries = match std::fs::read_dir(root.join(rel)) {
            Ok(entries) => entries,
            Err(e) if depth == 0 && e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("{rel}: {e}")),
        };
        for entry in entries {
            let entry = entry.map_err(|e| format!("{rel}: {e}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{rel}/{name}");
            let kind = entry.file_type().map_err(|e| format!("{path}: {e}"))?;
            if kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()) {
                if depth < 4 && !name.starts_with('.') {
                    walk(root, &path, depth + 1, files)?;
                }
            } else if name.ends_with(".md")
                && name
                    .split('-')
                    .next()
                    .is_some_and(|n| n.parse::<u32>().is_ok())
            {
                files.push(path);
            }
        }
        Ok(())
    }
    let mut members = Vec::new();
    let mut paths = Vec::new();
    walk(root, "efforts", 0, &mut paths)?;
    paths.sort();
    for path in paths {
        let text = std::fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
        let file = super::EffortFile::parse(&text).map_err(|e| format!("{path}: {e}"))?;
        if file.prop("initiative").is_some_and(|name| {
            let name = initiative_name(name);
            name == initiative_name(&page.slug) || name == initiative_name(&page.file.title)
        }) {
            members.push((path, file));
        }
    }
    Ok(members)
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    #[test]
    fn aliases_resolve_and_collisions_never_guess() {
        let root = std::env::temp_dir().join(format!(
            "initiative-catalog-{}",
            crate::activate::random_hex().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir(root.as_path().join("initiatives")).unwrap();
        let put = |slug: &str, title: &str| {
            std::fs::write(
                root.as_path().join(format!("initiatives/{slug}.md")),
                format!("# {title}\n"),
            )
            .unwrap()
        };
        put("smooth-lines", "Smooth ink");
        let pages = initiative_catalog(root.as_path()).unwrap();
        for alias in [" SMOOTH-LINES ", "[[Smooth ink]]", "[[ smooth-lines ]]"] {
            assert_eq!(
                resolve_initiative(&pages, alias).unwrap().slug,
                "smooth-lines"
            );
        }
        assert!(resolve_initiative(&pages, "missing").is_err());
        for title in ["SMOOTH INK", "Smooth-lines"] {
            put("other", title);
            assert!(
                initiative_catalog(root.as_path())
                    .unwrap_err()
                    .contains("ambiguous")
            );
        }
        std::fs::remove_file(root.as_path().join("initiatives/other.md")).unwrap();
        let mut collision = pages[0].clone();
        collision.slug = "SMOOTH-LINES".into();
        assert!(resolve_initiative(&[pages[0].clone(), collision], "smooth-lines").is_err());
    }

    #[test]
    fn synthetic_legacy_pages_keep_every_unmanaged_byte_on_repeated_saves() {
        for text in [
            include_str!("../../tests/fixtures/initiatives/prose.md"),
            include_str!("../../tests/fixtures/initiatives/status-only.md"),
        ] {
            let mut file = super::super::EffortFile::parse(text).unwrap();
            assert_eq!(file.render(), text);
            let unmanaged = text.split_once(&format!("# {}\n", file.title)).unwrap().1;
            for state in ["First checkpoint.", "Second checkpoint."] {
                file.current = state.into();
                let rendered = file.render_initiative();
                assert!(rendered.ends_with(unmanaged), "{rendered:?}");
                assert!(rendered.contains("## Decisions\n"));
                file = super::super::EffortFile::parse(&rendered).unwrap();
                assert!(
                    file.decisions.is_empty(),
                    "legacy purpose must not become Decisions: {:?}",
                    file.decisions
                );
            }
        }
        assert!(
            super::super::EffortFile::parse("# Page\n## Current state\na\n## Current state\nb\n")
                .unwrap_err()
                .contains("duplicate")
        );
    }
}

/// Joining changes only the membership property, retaining even legacy effort formatting.
/// In a header that never closes, a membership line already there is replaced and none is
/// added; filing refuses such a note before it gets here.
pub fn set_initiative_property(text: &str, slug: &str) -> String {
    let clean = text.strip_prefix('\u{feff}').unwrap_or(text);
    let bom = &text[..text.len() - clean.len()];
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let Some(at) = super::file::header_bounds(text) else {
        return format!("{bom}---{eol}initiative: {slug}{eol}---{eol}{clean}");
    };
    let mut out = text[..at.start].to_owned();
    let mut written = false;
    for line in text[at.start..at.end].split_inclusive('\n') {
        if !is_initiative_key(line) {
            out += line;
        } else if !written {
            out += &format!("initiative: {slug}{eol}");
            written = true;
        }
    }
    // A new line goes before the closing line; a header that never closes has none.
    if !written && at.closed() {
        out += &format!("initiative: {slug}{eol}");
    }
    out + &text[at.end..]
}

/// Whether a header line holds the initiative key as the shared header reader sees it:
/// any case, spaces around the key, on a line that isn't indented or a list item.
fn is_initiative_key(line: &str) -> bool {
    !line.starts_with(' ')
        && !line.trim_start().starts_with("- ")
        && line
            .split_once(':')
            .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("initiative"))
}

#[cfg(test)]
mod property_tests {
    use super::set_initiative_property;
    use crate::effort::file::frontmatter;

    fn initiative_lines(text: &str) -> usize {
        frontmatter(text)
            .iter()
            .filter(|(k, _)| k == "initiative")
            .count()
    }

    #[test]
    fn any_case_of_the_key_is_replaced_with_one_line() {
        for key in ["initiative", "Initiative", "INITIATIVE", "Initiative "] {
            let text =
                format!("---\nstatus: active\n{key}: old\nrepo: demo\n---\n# 0001 One\n\nBody.\n");
            let out = set_initiative_property(&text, "new");
            assert_eq!(
                out, "---\nstatus: active\ninitiative: new\nrepo: demo\n---\n# 0001 One\n\nBody.\n",
                "{key:?}"
            );
            assert_eq!(initiative_lines(&out), 1, "{out}");
        }
    }

    #[test]
    fn later_duplicates_in_any_case_are_dropped() {
        let text = "---\nInitiative: a\nstatus: active\ninitiative: b\n---\n# 0001 One\n";
        assert_eq!(
            set_initiative_property(text, "new"),
            "---\ninitiative: new\nstatus: active\n---\n# 0001 One\n"
        );
    }

    #[test]
    fn a_header_without_the_key_gains_one_line_and_keeps_its_bytes() {
        let text =
            "\u{feff}---\r\nStatus: active\r\nrepo: demo\r\n---\r\n# 0001 One\r\n\r\nBody.  \r\n";
        let out = set_initiative_property(text, "new");
        assert_eq!(
            out,
            "\u{feff}---\r\nStatus: active\r\nrepo: demo\r\ninitiative: new\r\n---\r\n# 0001 One\r\n\r\nBody.  \r\n"
        );
        assert_eq!(initiative_lines(&out), 1);
    }

    #[test]
    fn a_capitalized_key_keeps_crlf_and_the_byte_order_mark() {
        let text = "\u{feff}---\r\nInitiative: old\r\nstatus: active\r\n---\r\n# 0001 One\r\n";
        assert_eq!(
            set_initiative_property(text, "new"),
            "\u{feff}---\r\ninitiative: new\r\nstatus: active\r\n---\r\n# 0001 One\r\n"
        );
    }

    #[test]
    fn an_oddly_shaped_header_gains_or_replaces_only_the_membership_line() {
        for (text, joined) in [
            (
                "---  \nstatus: active\n---\n# 0001 One\n",
                "---  \nstatus: active\ninitiative: new\n---\n# 0001 One\n",
            ),
            (
                "--- \nInitiative: old\nstatus: active\n--- \n# 0001 One\n",
                "--- \ninitiative: new\nstatus: active\n--- \n# 0001 One\n",
            ),
            (
                "---\nstatus: active\n---\t\n# 0001 One\n\n---\nInitiative: body\n---\n",
                "---\nstatus: active\ninitiative: new\n---\t\n# 0001 One\n\n---\nInitiative: body\n---\n",
            ),
            (
                "---\n---\n# 0001 One\n",
                "---\ninitiative: new\n---\n# 0001 One\n",
            ),
            (
                "\u{feff}---  \r\nInitiative: old\r\n---  \r\n# 0001 One",
                "\u{feff}---  \r\ninitiative: new\r\n---  \r\n# 0001 One",
            ),
            (
                "---\nstatus: active\n---",
                "---\nstatus: active\ninitiative: new\n---",
            ),
            ("# 0001 One\n", "---\ninitiative: new\n---\n# 0001 One\n"),
            // A header that never closes: a line already there is replaced, even as the last
            // line with no line break, and none is added to one without it.
            (
                "---\nstatus: active\nInitiative: old",
                "---\nstatus: active\ninitiative: new\n",
            ),
            (
                "\u{feff}# 0001 One\r\n",
                "\u{feff}---\r\ninitiative: new\r\n---\r\n# 0001 One\r\n",
            ),
        ] {
            let out = set_initiative_property(text, "new");
            assert_eq!(out, joined, "{text:?}");
            assert_eq!(initiative_lines(&out), 1, "{out:?}");
            // Joining again changes nothing.
            assert_eq!(set_initiative_property(&out, "new"), out, "{text:?}");
        }
    }

    #[test]
    fn a_header_that_never_closes_gains_no_membership_line() {
        for text in [
            "---",
            "--- ",
            "\u{feff}---",
            "---\n",
            "---\nstatus: active\n# 0001 One\n",
        ] {
            assert_eq!(set_initiative_property(text, "new"), text, "{text:?}");
        }
    }

    #[test]
    fn body_lines_that_look_like_the_key_are_left_alone() {
        let text = "---\nstatus: active\n---\n# 0001 One\n\nInitiative: not a property\n";
        assert_eq!(
            set_initiative_property(text, "new"),
            "---\nstatus: active\ninitiative: new\n---\n# 0001 One\n\nInitiative: not a property\n"
        );
    }
}

#[cfg(test)]
mod preservation_tests {
    use super::super::EffortFile;

    #[test]
    fn an_existing_purpose_heading_is_reused_and_decisions_stay_separate() {
        let text = "---\r\nstatus: ready\r\nrepo: demo\r\n---\r\n# Curves\r\n\r\n## Purpose\r\n\r\nKeep  these bytes.  \r\n\r\n## Order\r\n[a](../a.md)\r\n";
        let mut file = EffortFile::parse(text).unwrap();
        assert_eq!(file.render(), text);
        let purpose = &text[text.find("## Purpose").unwrap()..];
        for state in ["First.", "Second."] {
            file.current = state.into();
            file.add_decision("2026-01-01: curves over corners: smoother.");
            let out = file.render_initiative();
            assert!(out.ends_with(purpose));
            assert_eq!(out.matches("## Purpose").count(), 1);
            file = EffortFile::parse(&out).unwrap();
            assert!(!file.decisions.contains("these bytes"));
        }
    }

    #[test]
    fn a_preamble_before_the_title_is_preserved_under_purpose_too() {
        let text = "```md\n## example\n```\n# Curves\n\nPurpose text.\n\n## Order\nOne.\n";
        let mut file = EffortFile::parse(text).unwrap();
        file.current = "Active.".into();
        let out = file.render_initiative();
        let back = EffortFile::parse(&out).unwrap();
        assert!(back.decisions.is_empty());
        assert!(out.contains("```md\n## example\n```\n"));
        assert!(out.contains("\nPurpose text.\n"));
        assert_eq!(out.matches("## Purpose").count(), 1);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;

    #[test]
    fn join_numbers_allow_punctuation_and_trailing_prose() {
        let page = InitiativePage::parse(
            "## Efforts\neffort 42.\n\neffort 43, the baseline\nAdditional explanation.\n",
        )
        .unwrap();
        assert_eq!(page.joins, vec![42, 43]);
        for invalid in ["effort .42", "effort 0.", "effort 999999999999999."] {
            assert!(InitiativePage::parse(&format!("## Efforts\n{invalid}\n")).is_err());
        }
    }

    #[test]
    fn saving_without_purpose_text_never_inserts_an_empty_purpose_section() {
        for text in [
            "# Empty\n",
            "# Empty\n\n## Current state\nActive.\n\n## Decisions\n\n## Order\nOne.\n",
        ] {
            let mut file = super::super::EffortFile::parse(text).unwrap();
            for state in ["First.", "Second."] {
                file.current = state.into();
                let rendered = file.render_initiative();
                assert!(!rendered.contains("## Purpose"), "{rendered}");
                file = super::super::EffortFile::parse(&rendered).unwrap();
            }
        }
        let page = InitiativePage::parse("## Efforts\nChild\nA goal.\n").unwrap();
        assert!(
            !page
                .render(
                    "Empty",
                    "demo",
                    "2026-01-01",
                    &[(1, "efforts/0001-child.md".into())]
                )
                .contains("## Purpose")
        );
    }
}
