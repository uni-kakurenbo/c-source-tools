use crate::modules::Graph;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Components, Path};

type Neighbors<'a> = BTreeMap<&'a Path, BTreeSet<&'a Path>>;

#[derive(Default)]
struct Directory<'a> {
    sources: Vec<&'a str>,
    children: Vec<(&'a OsStr, Self)>,
}

impl<'a> Directory<'a> {
    fn insert(&mut self, mut directories: Components<'a>, source: &'a str) {
        let Some(component) = directories.next() else {
            self.sources.push(source);
            return;
        };

        let name = component.as_os_str();
        let index = self.children.iter().position(|(child, _)| *child == name);
        let index = index.unwrap_or_else(|| {
            self.children.push((name, Self::default()));
            self.children.len() - 1
        });

        self.children[index].1.insert(directories, source);
    }

    fn append_sources(self, neighbors: &Neighbors<'a>, ordered: &mut Vec<&'a str>) {
        let mut remaining = self.sources;
        let mut adjacent = None;

        while !remaining.is_empty() {
            let index = adjacent
                .and_then(|linked: &BTreeSet<&Path>| {
                    remaining
                        .iter()
                        .position(|source| linked.contains(Path::new(source)))
                })
                .unwrap_or(0);

            let source = remaining.remove(index);
            ordered.push(source);
            adjacent = neighbors.get(Path::new(source));
        }

        for (_, child) in self.children {
            child.append_sources(neighbors, ordered);
        }
    }
}

fn local_neighbors(graph: &Graph) -> Neighbors<'_> {
    let sources: BTreeSet<_> = graph.sources.iter().map(Path::new).collect();
    let mut neighbors = Neighbors::new();

    for &source in &sources {
        let header = source.with_extension("h");

        for file in [source, &header] {
            let Some(includes) = file.to_str().and_then(|name| graph.includes.get(name)) else {
                continue;
            };

            for include in includes {
                if include.system
                    || !include.name.ends_with(".h")
                    || graph.generated.contains(&include.name)
                {
                    continue;
                }

                let implementation = Path::new(&include.name).with_extension("c");
                let Some(&target) = sources.get(implementation.as_path()) else {
                    continue;
                };

                if target != source && target.parent() == source.parent() {
                    neighbors.entry(source).or_default().insert(target);
                    neighbors.entry(target).or_default().insert(source);
                }
            }
        }
    }

    neighbors
}

pub(crate) fn ordered_sources(graph: &Graph) -> Vec<&str> {
    let mut root = Directory::default();

    for source in &graph.sources {
        let mut directories = Path::new(source).components();
        directories.next_back();
        root.insert(directories, source);
    }

    let mut ordered = Vec::with_capacity(graph.sources.len());
    root.append_sources(&local_neighbors(graph), &mut ordered);

    ordered
}
