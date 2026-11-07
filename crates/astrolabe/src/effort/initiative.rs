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
        let start = (0..lines.len())
            .find(|&i| heading_at(i).is_some_and(|h| h.to_lowercase().starts_with("efforts")))
            .ok_or("the body needs a `## Efforts` section naming the efforts in order, one paragraph each: the title on its first line, the goal on the lines below it")?;
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
        for paragraph in lines[start + 1..end].join("\n").split("\n\n") {
            let mut parts = paragraph.trim().lines().map(str::trim);
            let Some(title) = parts.next().filter(|t| !t.is_empty()) else {
                continue;
            };
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
        if efforts.is_empty() {
            return Err("the `## Efforts` section is empty: one paragraph per effort, the title on its first line, the goal below it".to_owned());
        }
        Ok(Self {
            before: before.join("\n").trim().to_owned(),
            heading: heading_at(start).unwrap_or("Efforts").to_owned(),
            efforts,
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
        out += &format!("\n## {}\n\n", self.heading);
        for (i, ((name, goal), (_, rel))) in self.efforts.iter().zip(filed).enumerate() {
            let goal = goal.split_whitespace().collect::<Vec<_>>().join(" ");
            out += &format!("{}. [{name}](../{rel}). {goal}\n", i + 1);
        }
        if !self.after.is_empty() {
            out += &format!("\n{}\n", self.after);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_the_page_and_its_efforts_section_names_the_efforts() {
        let body = "# Ink\n\nSmooth the ink.\n\n```\n## Efforts\nnot this one\n```\n\n## Efforts and order\n1. Tether cap\nCap the tether.\n\n- Goo at speed\nKeep goo stable\nat speed.\n\n## What completion means\n- never splits\n";
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
        assert_eq!(
            text,
            "---\nstatus: ready\nrepo: demo\ncreated: 2026-11-07\n---\n# Ink\n\nSmooth the ink.\n\n```\n## Efforts\nnot this one\n```\n\n## Efforts and order\n\n1. [Tether cap](../efforts/2026/11-November/0002-tether-cap.md). Cap the tether.\n2. [Goo at speed](../efforts/2026/11-November/0003-goo-at-speed.md). Keep goo stable at speed.\n\n## What completion means\n- never splits\n"
        );
        for (body, why) in [
            ("Purpose only.", "`## Efforts` section"),
            ("## Efforts\n\n## Done\n", "is empty"),
            ("## Efforts\nTitle only\n", "has no goal"),
        ] {
            let err = InitiativePage::parse(body).unwrap_err();
            assert!(err.contains(why), "{body:?}: {err}");
        }
    }
}
