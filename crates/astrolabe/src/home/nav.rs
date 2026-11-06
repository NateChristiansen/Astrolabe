//! The notes tree, shelf ordering and effort labels.
use super::FILE;
use super::render::{Links, esc, file_url};
use crate::board::{BoardData, EffortRow};
use crate::notes::{self};
use std::path::Path;

/// A folder of notes as the tree shows it.
pub(super) struct Node {
    name: String,
    path: String,
    dirs: Vec<Node>,
    files: Vec<String>,
    open: bool,
}

impl Node {
    fn new(name: &str, path: &str) -> Node {
        Node {
            name: name.to_owned(),
            path: path.to_owned(),
            dirs: Vec::new(),
            files: Vec::new(),
            open: false,
        }
    }

    fn insert(&mut self, rel: &str) {
        let inner = rel.strip_prefix(&self.path).unwrap_or(rel);
        let inner = inner.trim_start_matches('/');
        match inner.split_once('/') {
            Some((dir, _)) => {
                let path = format!("{}/{dir}", self.path);
                let child = match self.dirs.iter_mut().position(|d| d.name == dir) {
                    Some(i) => &mut self.dirs[i],
                    None => {
                        self.dirs.push(Node::new(dir, &path));
                        self.dirs.last_mut().unwrap()
                    }
                };
                child.insert(rel);
            }
            None => self.files.push(rel.to_owned()),
        }
    }

    fn sort(&mut self, newest_first: bool) {
        self.dirs.sort_by(|a, b| a.name.cmp(&b.name));
        self.files.sort();
        if newest_first {
            self.dirs.reverse();
            self.files.reverse();
        }
        for d in &mut self.dirs {
            d.sort(newest_first);
        }
    }

    /// Open down the first branch: the newest month of this year's efforts.
    fn open_latest(&mut self) {
        self.open = true;
        if let Some(first) = self.dirs.first_mut() {
            first.open_latest();
        }
    }

    fn count(&self) -> usize {
        self.files.len() + self.dirs.iter().map(Node::count).sum::<usize>()
    }
}

pub(super) fn tree_html(
    root: &Path,
    rows: &BoardData,
    links: Links,
    current: Option<&str>,
) -> String {
    const FIRST: [&str; 5] = ["efforts", "journal", "knowledge", "lessons", "drafts"];
    let mut names: Vec<(bool, String)> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            if name == ".git" || name == ".obsidian" {
                continue;
            }
            names.push((name.starts_with('.'), name));
        } else if name.ends_with(".md") {
            files.push(name);
        }
    }
    names.sort_by_key(|(hidden, name)| {
        let rank = FIRST
            .iter()
            .position(|f| f == name)
            .unwrap_or(FIRST.len() + usize::from(*hidden));
        (rank, name.clone())
    });
    files.sort();
    let by_file: std::collections::HashMap<&str, &EffortRow> = rows
        .efforts
        .iter()
        .filter_map(|e| e.file.as_deref().map(|f| (f, e)))
        .collect();
    let mut shown = String::new();
    let mut hidden = String::new();
    let mut hidden_names = Vec::new();
    let mut hidden_count = 0;
    for (is_hidden, name) in &names {
        let mut node = Node::new(name, name);
        let mut any = false;
        for rel in notes::md_files(root, name) {
            node.insert(&rel);
            any = true;
        }
        if !any {
            continue;
        }
        let newest_first = matches!(name.as_str(), "efforts" | "journal" | "lessons");
        node.sort(newest_first);
        if matches!(name.as_str(), "efforts" | "journal") {
            node.open_latest();
        }
        let html = dir_html(&node, 0, None, &by_file, root, links, current);
        if *is_hidden {
            hidden += &html;
            hidden_names.push(name.clone());
            hidden_count += node.count();
        } else {
            shown += &html;
        }
    }
    let board_on = if current.is_none() { " on" } else { "" };
    let mut out = format!(
        "<a class=\"top{board_on}\" href=\"{}\"><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><rect x=\"3\" y=\"4\" width=\"18\" height=\"16\" rx=\"2\"/><path d=\"M9 4v16M15 4v16\"/></svg><span class=\"t\">Board</span></a>\n",
        match links {
            Links::Served => "/".to_owned(),
            Links::File => file_url(&root.join(FILE)),
        }
    );
    out += &shown;
    for name in &files {
        out += &file_row(name, 0, &by_file, root, links, current);
    }
    if !hidden.is_empty() {
        out += &format!(
            "<div class=\"hid\" id=\"hid\" hidden>{hidden}</div><button class=\"hidden-toggle\" id=\"hidtoggle\" data-names=\"{}\" data-n=\"{hidden_count}\">Show {} · {hidden_count} note{} agents wrote for each other</button>\n",
            esc(&hidden_names.join(", ")),
            esc(&hidden_names.join(", ")),
            if hidden_count == 1 { "" } else { "s" }
        );
    }
    out
}

/// `2026`, a shelf of months: its months are listed in its place, each carrying the year.
pub(super) fn is_year(name: &str) -> bool {
    name.len() == 4 && name.chars().all(|c| c.is_ascii_digit())
}

/// `11-November` reads as `November`; a filed note's own number stays on its row.
pub(super) fn label_of(name: &str) -> &str {
    name.split_once('-')
        .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .map_or(name, |(_, rest)| rest)
}

pub(super) fn dir_html(
    node: &Node,
    depth: usize,
    year: Option<&str>,
    by_file: &std::collections::HashMap<&str, &EffortRow>,
    root: &Path,
    links: Links,
    current: Option<&str>,
) -> String {
    if is_year(&node.name) && node.files.is_empty() {
        return node
            .dirs
            .iter()
            .map(|d| dir_html(d, depth, Some(&node.name), by_file, root, links, current))
            .collect();
    }
    let label = match year {
        Some(year) => format!("{} {year}", label_of(&node.name)),
        None => label_of(&node.name).to_owned(),
    };
    let mut out = format!(
        "<details{} data-key=\"{}\"><summary style=\"--d:{depth}\"><span class=\"chev\">›</span><span class=\"t\">{}</span><span class=\"n\">{}</span></summary>\n",
        if node.open { " open" } else { "" },
        esc(&node.path),
        esc(&label),
        node.count()
    );
    for d in &node.dirs {
        out += &dir_html(d, depth + 1, None, by_file, root, links, current);
    }
    for f in &node.files {
        out += &file_row(f, depth + 1, by_file, root, links, current);
    }
    out + "</details>\n"
}

pub(super) fn file_row(
    rel: &str,
    depth: usize,
    by_file: &std::collections::HashMap<&str, &EffortRow>,
    root: &Path,
    links: Links,
    current: Option<&str>,
) -> String {
    let on = if current == Some(rel) { " on" } else { "" };
    let href = esc(&links.href(root, rel));
    match by_file.get(rel) {
        Some(e) => {
            let status = if e.needs_owner {
                "needs".to_owned()
            } else {
                e.status.as_deref().unwrap_or_default().to_owned()
            };
            let title = e.title.as_deref().unwrap_or_default();
            format!(
                "<a class=\"f{on}\" style=\"--d:{depth}\" href=\"{href}\" title=\"{}\" data-rel=\"{}\"><span class=\"st s-{}\"></span><span class=\"num\">{}</span><span class=\"t\">{}</span></a>\n",
                esc(title),
                esc(rel),
                esc(&status),
                e.number.unwrap_or(0),
                esc(title)
            )
        }
        None => {
            let name = rel
                .rsplit('/')
                .next()
                .unwrap_or(rel)
                .trim_end_matches(".md");
            // `05-Thursday` reads as the day's number and its name.
            let (num, text) = match name.split_once('-') {
                Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
                    (format!("<span class=\"num\">{}</span>", esc(n)), rest)
                }
                _ => (String::new(), name),
            };
            format!(
                "<a class=\"f{on}\" style=\"--d:{depth}\" href=\"{href}\" title=\"{}\" data-rel=\"{}\">{num}<span class=\"t\">{}</span></a>\n",
                esc(name),
                esc(rel),
                esc(text)
            )
        }
    }
}
