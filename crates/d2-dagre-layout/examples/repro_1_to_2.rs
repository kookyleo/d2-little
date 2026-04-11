//! Reproduce the dagre-rs layout for `a -> b; a -> c` to compare against
//! Go d2's bundled dagre.js v0.8.5.
//!
//! Expected (Go d2 / dagre.js v0.8.5):
//!   node 0: x=83 y=33
//!   node 1: x=26.5 y=199   ← b on the LEFT
//!   node 2: x=139.5 y=199  ← c on the RIGHT
//!
//! What we (dagre-rs) currently produce will be printed; if it differs,
//! that's the byte-divergence we need to chase down for the e2e suite.

fn main() {
    let graph_opts = dagre::graph::GraphOptions {
        directed: true,
        multigraph: true,
        compound: true,
    };
    let mut g = dagre::graph::Graph::<dagre::NodeLabel, dagre::EdgeLabel>::with_options(graph_opts);
    g.set_graph_label(dagre::GraphLabel {
        compound: true,
        rankdir: dagre::layout::types::RankDir::TB,
        nodesep: 60.0,
        edgesep: 40.0,
        ranksep: 100.0,
        ..Default::default()
    });
    g.set_node(
        "0".to_owned(),
        Some(dagre::NodeLabel {
            width: 53.0,
            height: 66.0,
            ..Default::default()
        }),
    );
    g.set_node(
        "1".to_owned(),
        Some(dagre::NodeLabel {
            width: 53.0,
            height: 66.0,
            ..Default::default()
        }),
    );
    g.set_node(
        "2".to_owned(),
        Some(dagre::NodeLabel {
            width: 53.0,
            height: 66.0,
            ..Default::default()
        }),
    );
    g.set_edge(
        "0".to_owned(),
        "1".to_owned(),
        Some(dagre::EdgeLabel {
            width: 0.0,
            height: 0.0,
            labelpos: dagre::layout::types::LabelPos::Center,
            ..Default::default()
        }),
        Some("(a -> b)[0]"),
    );
    g.set_edge(
        "0".to_owned(),
        "2".to_owned(),
        Some(dagre::EdgeLabel {
            width: 0.0,
            height: 0.0,
            labelpos: dagre::layout::types::LabelPos::Center,
            ..Default::default()
        }),
        Some("(a -> c)[0]"),
    );

    dagre::layout(
        &mut g,
        Some(dagre::LayoutOptions {
            rankdir: dagre::layout::types::RankDir::TB,
            nodesep: 60.0,
            edgesep: 40.0,
            ranksep: 100.0,
            ..Default::default()
        }),
    );

    for id in g.nodes() {
        let n = g.node(&id).unwrap();
        println!(
            "node {}: x={:?} y={:?}",
            id,
            n.x.unwrap_or(0.0),
            n.y.unwrap_or(0.0)
        );
    }
}
