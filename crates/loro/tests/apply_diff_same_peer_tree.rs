//! `apply_diff` of a diff onto a document of the same peer whose next counters are the ones the
//! diff's new containers have: replaying local changes on a rebuilt copy, as an app does after its
//! history was rebuilt. A tree node that the target does not have is created under the target's
//! next id, which can be the very id the diff names, and the remap must not loop.

use std::{sync::mpsc, thread, time::Duration};

use loro::{LoroDoc, LoroTree, TreeParentId};

/// Runs `f` on another thread and fails if it does not finish in time (a remap that loops would
/// spin for ever).
fn finishes(f: impl FnOnce() + Send + 'static) {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        f();
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(20))
        .expect("apply_diff did not finish: the container remap loops");
}

fn doc(peer: u64) -> LoroDoc {
    let d = LoroDoc::new();
    d.set_peer_id(peer).unwrap();
    d
}

#[test]
fn a_tree_node_created_under_the_id_the_diff_names_is_applied() {
    finishes(|| {
        let old = doc(7);
        let tree: LoroTree = old.get_tree("tree");
        let root = tree.create(TreeParentId::Root).unwrap();
        old.commit();
        let at = old.state_frontiers();
        // The rebuilt copy: the same peer, at the same version, so its next counter is the one
        // the node below gets in `old`.
        let rebuilt = old.fork_at(&at).unwrap();
        rebuilt.set_peer_id(7).unwrap();
        let child = tree.create(root).unwrap();
        tree.get_meta(child).unwrap().insert("k", "v").unwrap();
        old.commit();
        let diff = old.diff(&at, &old.state_frontiers()).unwrap();
        rebuilt.apply_diff(diff).unwrap();
        rebuilt.commit();
        assert_eq!(
            rebuilt.get_deep_value(),
            old.get_deep_value()
        );
    });
}

#[test]
fn two_tree_nodes_whose_new_ids_swap_are_applied() {
    finishes(|| {
        let old = doc(7);
        let tree: LoroTree = old.get_tree("tree");
        let root = tree.create(TreeParentId::Root).unwrap();
        old.commit();
        let at = old.state_frontiers();
        let rebuilt = old.fork_at(&at).unwrap();
        rebuilt.set_peer_id(7).unwrap();
        // The first node is created first, the second is moved before it: the diff may list them
        // in another order than their counters.
        let a = tree.create(root).unwrap();
        let b = tree.create(root).unwrap();
        tree.mov_before(b, a).unwrap();
        tree.get_meta(a).unwrap().insert("n", "a").unwrap();
        tree.get_meta(b).unwrap().insert("n", "b").unwrap();
        old.commit();
        let diff = old.diff(&at, &old.state_frontiers()).unwrap();
        rebuilt.apply_diff(diff).unwrap();
        rebuilt.commit();
        let names = |d: &LoroDoc| {
            let t = d.get_tree("tree");
            t.children(TreeParentId::Node(root))
                .unwrap_or_default()
                .into_iter()
                .map(|n| t.get_meta(n).unwrap().get("n").unwrap().into_value().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&rebuilt), names(&old));
    });
}
