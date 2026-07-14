use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, serde::Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    resolve: Option<Resolve>,
}

#[derive(Debug, serde::Deserialize)]
struct Package {
    name: String,
    id: String,
}

#[derive(Debug, serde::Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Debug, serde::Deserialize)]
struct Node {
    id: String,
    dependencies: Vec<String>,
}

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut dir = manifest_dir.as_path();
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if let Ok(content) = std::fs::read_to_string(&cargo_toml)
            && content.contains("[workspace]")
        {
            return dir.to_path_buf();
        }
        let Some(parent) = dir.parent() else {
            panic!("workspace root not found");
        };
        dir = parent;
    }
}

fn load_metadata() -> Metadata {
    let output = match Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .current_dir(workspace_root())
        .output()
    {
        Ok(o) => o,
        Err(e) => panic!("cargo metadata failed to run: {e}"),
    };
    assert!(output.status.success(), "cargo metadata exited with error");
    match serde_json::from_slice::<Metadata>(&output.stdout) {
        Ok(m) => m,
        Err(e) => panic!("cargo metadata JSON parse error: {e}"),
    }
}

fn workspace_members(metadata: &Metadata) -> Vec<&Package> {
    metadata
        .packages
        .iter()
        .filter(|p| p.name.starts_with("cheetah-"))
        .collect()
}

fn resolve_nodes(metadata: &Metadata) -> &[Node] {
    let Some(resolve) = metadata.resolve.as_ref() else {
        panic!("metadata resolve missing");
    };
    &resolve.nodes
}

fn transitive_dependencies<'m>(metadata: &'m Metadata, package: &Package) -> BTreeSet<&'m str> {
    let mut visited = BTreeSet::new();
    let mut stack = vec![package.id.as_str()];
    let nodes = resolve_nodes(metadata);

    while let Some(current_id) = stack.pop() {
        if !visited.insert(current_id) {
            continue;
        }
        let Some(node) = nodes.iter().find(|n| n.id == current_id) else {
            continue;
        };
        for dep_id in &node.dependencies {
            stack.push(dep_id.as_str());
        }
    }

    visited
        .iter()
        .filter_map(|id| metadata.packages.iter().find(|p| p.id == *id))
        .map(|p| p.name.as_str())
        .collect()
}

fn assert_not_depend(metadata: &Metadata, package_name: &str, forbidden: &HashSet<&str>) {
    let Some(package) = metadata.packages.iter().find(|p| p.name == package_name) else {
        panic!("package {package_name} not found");
    };
    let deps = transitive_dependencies(metadata, package);
    for bad in forbidden {
        assert!(
            !deps.contains(*bad),
            "{package_name} must not depend on {bad}"
        );
    }
}

#[test]
fn domain_crates_do_not_depend_on_runtime_or_adapters() {
    let metadata = load_metadata();
    let forbidden: HashSet<&str> = ["tokio", "axum", "tonic", "sqlx", "async-nats", "quick-xml"]
        .into_iter()
        .collect();

    assert_not_depend(&metadata, "cheetah-domain", &forbidden);
}

#[test]
fn protocol_core_crates_do_not_depend_on_runtime_or_io() {
    let metadata = load_metadata();
    let forbidden: HashSet<&str> = [
        "tokio",
        "socket2",
        "reqwest",
        "hyper",
        "sqlx",
        "async-nats",
        "cheetah-media-client",
    ]
    .into_iter()
    .collect();

    for package in workspace_members(&metadata) {
        if package.name.ends_with("-core") {
            assert_not_depend(&metadata, &package.name, &forbidden);
        }
    }
}

fn visit<'a>(
    node: &'a str,
    graph: &BTreeMap<&'a str, BTreeSet<&'a str>>,
    visiting: &mut HashSet<&'a str>,
    visited: &mut HashSet<&'a str>,
) {
    if visited.contains(node) {
        return;
    }
    assert!(
        visiting.insert(node),
        "dependency cycle detected involving {node}"
    );
    for child in graph.get(node).cloned().unwrap_or_default() {
        visit(child, graph, visiting, visited);
    }
    visiting.remove(node);
    visited.insert(node);
}

#[test]
fn crate_graph_is_acyclic() {
    let metadata = load_metadata();
    let members: BTreeSet<&str> = workspace_members(&metadata)
        .into_iter()
        .map(|p| p.name.as_str())
        .collect();

    let nodes = resolve_nodes(&metadata);
    let mut graph: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for node in nodes {
        let Some(pkg) = metadata.packages.iter().find(|p| p.id == node.id) else {
            continue;
        };
        if !members.contains(pkg.name.as_str()) {
            continue;
        }
        let edges: BTreeSet<&str> = node
            .dependencies
            .iter()
            .filter_map(|dep_id| metadata.packages.iter().find(|p| p.id == *dep_id))
            .filter(|p| members.contains(p.name.as_str()))
            .map(|p| p.name.as_str())
            .collect();
        graph.insert(pkg.name.as_str(), edges);
    }

    let mut visited = HashSet::new();
    for node in graph.keys().copied() {
        let mut visiting = HashSet::new();
        visit(node, &graph, &mut visiting, &mut visited);
    }
}
