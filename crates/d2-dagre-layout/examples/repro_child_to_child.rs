//! Reproduce the dagre-rs raw output for `a.b -> c.d` so we can see what
//! container sizing looks like *before* any post-processing. Compare against
//! Go d2's pre-fitContainerPadding state to figure out how much fitPadding
//! needs to do.

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
    // Mirror what d2-dagre-layout would feed for `a.b -> c.d`:
    // four objects total (a, a.b, c, c.d), two containers (a, c).
    // Leaf widths come from label measurement: "a" → 8 + 5 + 40 = 53
    // wide, height = 21 + 5 + 40 = 66 high. Containers get the same
    // baseline since they only have a label "a"/"c".
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
    g.set_node(
        "3".to_owned(),
        Some(dagre::NodeLabel {
            width: 54.0,
            height: 66.0,
            ..Default::default()
        }),
    );
    g.set_parent("1", Some("0"));
    g.set_parent("3", Some("2"));
    g.set_edge(
        "1".to_owned(),
        "3".to_owned(),
        Some(dagre::EdgeLabel {
            width: 0.0,
            height: 0.0,
            labelpos: dagre::layout::types::LabelPos::Center,
            ..Default::default()
        }),
        Some("(a.b -> c.d)[0]"),
    );

    dagre::layout(
        &mut g,
        Some(dagre::LayoutOptions {
            rankdir: dagre::layout::types::RankDir::TB,
            nodesep: 60.0,
            edgesep: 40.0,
            ranksep: 100.0,
            tie_keep_first: true,
            ..Default::default()
        }),
    );

    println!("nodes:");
    for id in g.nodes() {
        let n = g.node(&id).unwrap();
        let cx = n.x.unwrap_or(0.0);
        let cy = n.y.unwrap_or(0.0);
        let w = n.width;
        let h = n.height;
        let tlx = cx - w / 2.0;
        let tly = cy - h / 2.0;
        println!(
            "  id={} center=({:.1},{:.1}) size=({:.0}x{:.0}) topleft=({:.1},{:.1})",
            id, cx, cy, w, h, tlx, tly
        );
    }
    println!("edges:");
    for e in g.edges() {
        let el = g.edge_by_obj(&e).unwrap();
        println!("  {}->{} points={:?}", e.v, e.w, el.points);
    }
}
