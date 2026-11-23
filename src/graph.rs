use crate::workspace::Workspace;
use std::collections::{HashMap, HashSet};

/// The workspace's packages and the dependencies between them, by name.
pub struct DependencyGraph {
    /// Each package's dependencies within the workspace.
    dependencies: HashMap<String, Vec<String>>,
    /// Each package's direct dependents within the workspace.
    dependents: HashMap<String, Vec<String>>,
}

impl DependencyGraph {
    pub fn from_workspace(workspace: &Workspace) -> Self {
        let names: Vec<&str> = workspace.packages.iter().map(|p| p.name.as_str()).collect();
        let edges = workspace.packages.iter().flat_map(|package| {
            package
                .dependencies
                .iter()
                .map(move |dep| (package.name.as_str(), dep.as_str()))
        });
        Self::from_edges(&names, edges)
    }

    /// A graph of `packages` where each `(from, to)` edge means `from` depends
    /// on `to`. Edges to packages outside `packages` are dropped.
    fn from_edges<'a>(
        packages: &[&str],
        edges: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        let mut dependencies: HashMap<String, Vec<String>> = packages
            .iter()
            .map(|name| (name.to_string(), Vec::new()))
            .collect();
        let mut dependents = dependencies.clone();

        for (from, to) in edges {
            if !dependencies.contains_key(to) {
                continue;
            }
            if let Some(deps) = dependencies.get_mut(from) {
                deps.push(to.to_string());
                dependents
                    .get_mut(to)
                    .expect("every package has an entry")
                    .push(from.to_string());
            }
        }

        Self {
            dependencies,
            dependents,
        }
    }

    /// The packages that depend on `package` directly.
    pub fn dependents(&self, package: &str) -> Vec<String> {
        self.dependents.get(package).cloned().unwrap_or_default()
    }

    /// The packages that depend on `package`, directly or through others.
    pub fn all_dependents(&self, package: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut stack = vec![package];
        let mut result = Vec::new();

        while let Some(name) = stack.pop() {
            for dependent in self.dependents.get(name).into_iter().flatten() {
                if visited.insert(dependent.as_str()) {
                    result.push(dependent.clone());
                    stack.push(dependent);
                }
            }
        }

        result
    }

    /// The packages `package` depends on directly.
    pub fn dependencies(&self, package: &str) -> Vec<String> {
        self.dependencies.get(package).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(packages: &[&str], edges: &[(&'static str, &'static str)]) -> DependencyGraph {
        DependencyGraph::from_edges(packages, edges.iter().copied())
    }

    #[test]
    fn test_dependents() {
        let dep_graph = graph(&["a", "b", "c"], &[("b", "a"), ("c", "a")]);

        let dependents = dep_graph.dependents("a");
        assert_eq!(dependents.len(), 2);
        assert!(dependents.contains(&"b".to_string()));
        assert!(dependents.contains(&"c".to_string()));
    }

    #[test]
    fn test_all_dependents_transitive_chain() {
        let dep_graph = graph(&["a", "b", "c"], &[("b", "a"), ("c", "b")]);

        let all = dep_graph.all_dependents("a");
        assert_eq!(all.len(), 2);
        assert!(all.contains(&"b".to_string()));
        assert!(all.contains(&"c".to_string()));
    }

    #[test]
    fn test_all_dependents_diamond() {
        let dep_graph = graph(
            &["a", "b", "c", "d"],
            &[("b", "a"), ("c", "a"), ("d", "b"), ("d", "c")],
        );

        let all = dep_graph.all_dependents("a");
        assert_eq!(all.len(), 3);
        assert!(all.contains(&"b".to_string()));
        assert!(all.contains(&"c".to_string()));
        assert!(all.contains(&"d".to_string()));
    }

    #[test]
    fn test_all_dependents_cycle() {
        let dep_graph = graph(&["a", "b"], &[("a", "b"), ("b", "a")]);

        let mut all = dep_graph.all_dependents("a");
        all.sort();
        assert_eq!(all, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn test_dependencies() {
        let dep_graph = graph(&["a", "b", "c"], &[("b", "a"), ("c", "a")]);

        assert_eq!(dep_graph.dependencies("b"), vec!["a".to_string()]);
        assert!(dep_graph.dependencies("a").is_empty());
    }

    #[test]
    fn test_external_dependencies_are_dropped() {
        let dep_graph = graph(&["a"], &[("a", "serde")]);

        assert!(dep_graph.dependencies("a").is_empty());
        assert!(dep_graph.dependents("serde").is_empty());
    }

    #[test]
    fn test_nonexistent_package() {
        let dep_graph = graph(&["a"], &[]);

        assert!(dep_graph.dependents("nonexistent").is_empty());
        assert!(dep_graph.all_dependents("nonexistent").is_empty());
        assert!(dep_graph.dependencies("nonexistent").is_empty());
    }

    #[test]
    fn test_no_dependents() {
        let dep_graph = graph(&["a", "b"], &[("b", "a")]);

        assert!(dep_graph.dependents("b").is_empty());
    }
}
