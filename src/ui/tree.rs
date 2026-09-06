//! Builds the collapsible topic tree from the store.

use crate::nt::store::Store;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone)]
pub struct TreeRow {
    pub depth: usize,
    /// Directory name or topic leaf name.
    pub label: String,
    /// Full path (directory prefix or topic name).
    pub path: String,
    pub is_topic: bool,
}

struct Node {
    dirs: BTreeMap<String, Node>,
    topics: Vec<String>,
}

impl Node {
    fn new() -> Self {
        Node {
            dirs: BTreeMap::new(),
            topics: Vec::new(),
        }
    }
}

fn insert_topic(root: &mut Node, topic: &str) {
    // NT4 topic names are absolute ("/a/b/c"); strip the leading slash.
    let topic = topic.strip_prefix('/').unwrap_or(topic);
    let parts: Vec<&str> = topic.split('/').collect();
    let mut node = root;
    for part in &parts[..parts.len() - 1] {
        node = node.dirs.entry(part.to_string()).or_insert_with(Node::new);
    }
    node.topics.push(parts[parts.len() - 1].to_string());
}

fn walk(
    node: &Node,
    prefix: &str,
    depth: usize,
    expanded: &HashSet<String>,
    out: &mut Vec<TreeRow>,
) {
    for (name, child) in &node.dirs {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        let is_open = expanded.contains(&path);
        out.push(TreeRow {
            depth,
            label: if is_open {
                format!("[-] {}", name)
            } else {
                format!("[+] {}", name)
            },
            path: path.clone(),
            is_topic: false,
        });
        if is_open {
            walk(child, &path, depth + 1, expanded, out);
        }
    }
    for topic in node.topics.iter() {
        let path = if prefix.is_empty() {
            topic.clone()
        } else {
            format!("{}/{}", prefix, topic)
        };
        out.push(TreeRow {
            depth,
            label: topic.clone(),
            path,
            is_topic: true,
        });
    }
}

/// Flat row list for rendering. Rebuilt per frame; topic counts in the pit
/// are small (hundreds) so this stays well under a millisecond.
pub fn build_tree(store: &Store, expanded: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = Node::new();
    for name in store.sorted_names() {
        insert_topic(&mut root, &name);
    }
    let mut rows = Vec::new();
    walk(&root, "", 0, expanded, &mut rows);
    rows
}
