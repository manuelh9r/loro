//! Whether a root container is part of a doc's value must depend on its history
//! only: a root touched by any op is visible, even when its ops add up to an empty
//! value. A peer and a fresh replay of its own history used to disagree for such
//! roots. See loro-dev/loro#1156.

use loro::{ExportMode, LoroDoc, LoroMap, LoroText, ToJson, UndoManager, ValueOrContainer};
use pretty_assertions::assert_eq;
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde_json::json;

fn replay(doc: &LoroDoc) -> LoroDoc {
    let replay = LoroDoc::new();
    replay
        .import(&doc.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    replay
}

fn value(doc: &LoroDoc) -> serde_json::Value {
    doc.get_deep_value().to_json_value()
}

fn inserted_then_deleted() -> LoroDoc {
    let a = LoroDoc::new();
    a.set_peer_id(1).unwrap();
    a.get_text("t").insert(0, "ab").unwrap();
    a.commit();
    a.get_text("t").delete(0, 2).unwrap();
    a.commit();
    a
}

#[test]
fn issue_repro_empty_root_text_is_visible_after_replay() {
    let a = inserted_then_deleted();
    assert_eq!(value(&a), json!({"t": ""}));
    assert_eq!(value(&replay(&a)), json!({"t": ""}));
}

#[test]
fn every_import_path_agrees() {
    let a = inserted_then_deleted();
    let expected = value(&a);
    let updates = a.export(ExportMode::all_updates()).unwrap();

    // One import, a snapshot, a batch, JSON, and a detached import then attach.
    let snapshot = LoroDoc::new();
    snapshot
        .import(&a.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    assert_eq!(value(&snapshot), expected, "snapshot");

    let batch = LoroDoc::new();
    let first = LoroDoc::new();
    first.set_peer_id(9).unwrap();
    first.get_map("other").insert("k", 1).unwrap();
    first.commit();
    batch
        .import_batch(&[
            updates.clone(),
            first.export(ExportMode::all_updates()).unwrap(),
        ])
        .unwrap();
    let mut with_other = expected.clone();
    with_other["other"] = json!({"k": 1});
    assert_eq!(value(&batch), with_other, "import_batch");

    let from_json = LoroDoc::new();
    from_json
        .import_json_updates(
            a.export_json_updates(&Default::default(), &a.oplog_vv())
                .unwrap(),
        )
        .unwrap();
    assert_eq!(value(&from_json), expected, "json");

    let detached = LoroDoc::new();
    detached.detach();
    detached.import(&updates).unwrap();
    detached.try_attach().unwrap();
    assert_eq!(value(&detached), expected, "detached import + attach");

    // Into a doc that already has content of its own.
    let busy = LoroDoc::new();
    busy.set_peer_id(5).unwrap();
    busy.get_list("l").push(1).unwrap();
    busy.commit();
    busy.import(&updates).unwrap();
    let mut with_list = expected.clone();
    with_list["l"] = json!([1]);
    assert_eq!(value(&busy), with_list, "non-empty target");
}

#[test]
fn root_map_and_list_that_end_empty_are_visible() {
    let a = LoroDoc::new();
    a.set_peer_id(1).unwrap();
    a.get_map("m").insert("k", 1).unwrap();
    a.get_list("l").push(1).unwrap();
    a.get_movable_list("ml").push(1).unwrap();
    a.commit();
    a.get_map("m").delete("k").unwrap();
    a.get_list("l").delete(0, 1).unwrap();
    a.get_movable_list("ml").delete(0, 1).unwrap();
    a.commit();
    assert_eq!(value(&replay(&a)), value(&a));
}

/// The insert and the delete arrive in separate imports, or the insert is undone.
#[test]
fn separate_imports_and_undo_agree_with_replay() {
    let a = LoroDoc::new();
    a.set_peer_id(1).unwrap();
    let mut undo = UndoManager::new(&a);
    a.get_text("t").insert(0, "ab").unwrap();
    a.commit();
    undo.undo().unwrap();
    assert_eq!(value(&a), json!({"t": ""}));
    assert_eq!(value(&replay(&a)), value(&a));

    let src = inserted_then_deleted();
    let b = LoroDoc::new();
    let insert_only = loro::IdSpan::new(1, 0, 2);
    b.import(
        &src.export(ExportMode::updates_in_range(vec![insert_only]))
            .unwrap(),
    )
    .unwrap();
    b.import(&src.export(ExportMode::updates(&b.oplog_vv())).unwrap())
        .unwrap();
    assert_eq!(value(&b), json!({"t": ""}));
    assert_eq!(value(&b), value(&replay(&src)));
}

// A trimmed copy of the random workload from the issue: several peers editing a
// movable list, a text and a map, with undo/redo, syncs, snapshots, restarts and
// batches. Every non-shallow peer must equal a fresh replay of its own history.
//
// Getting a root handle (`doc.get_text("t")`) makes the root visible by design,
// with or without ops, and the workload reads roots to pick positions. So every
// peer first writes one op to each root: the roots it reads are then also in its
// history, and undo can still bring them back to empty values. This is a guard
// against new mismatches; the targeted tests above are the reproductions.

struct Peer {
    doc: LoroDoc,
    undo: UndoManager,
    shallow: bool,
}

fn new_peer(doc: LoroDoc, shallow: bool) -> Peer {
    let undo = UndoManager::new(&doc);
    Peer { doc, undo, shallow }
}

fn step(rng: &mut StdRng, peers: &mut [Peer], i: usize) {
    let n = peers.len();
    let p = rng.gen_range(0..n);
    let q = (p + rng.gen_range(1..n)) % n;
    let kind = rng.gen_range(0..16);
    let d = &peers[p].doc;
    let l = d.get_movable_list("l");
    let len = l.len();
    match kind {
        0 | 1 => {
            l.insert(rng.gen_range(0..=len), i as i64).unwrap();
        }
        2 if len > 1 => {
            l.mov(rng.gen_range(0..len), rng.gen_range(0..len)).unwrap();
        }
        3 if len > 0 => {
            l.set(rng.gen_range(0..len), i as i64).unwrap();
        }
        4 if len > 0 => {
            l.delete(rng.gen_range(0..len), 1).unwrap();
        }
        5 => {
            if rng.gen_bool(0.5) {
                let m = l
                    .insert_container(rng.gen_range(0..=len), LoroMap::new())
                    .unwrap();
                m.insert("v", i as i64).unwrap();
            } else {
                let t = l
                    .insert_container(rng.gen_range(0..=len), LoroText::new())
                    .unwrap();
                t.insert(0, "hi").unwrap();
            }
        }
        6 if len > 0 => {
            if let Some(ValueOrContainer::Container(c)) = l.get(rng.gen_range(0..len)) {
                match c {
                    loro::Container::Map(m) => {
                        m.insert("v", i as i64).unwrap();
                    }
                    loro::Container::Text(t) => {
                        let tl = t.len_unicode();
                        t.insert(rng.gen_range(0..=tl), "x").unwrap();
                    }
                    _ => {}
                }
            }
        }
        7 => {
            let t = d.get_text("t");
            let tl = t.len_unicode();
            t.insert(rng.gen_range(0..=tl), "ab").unwrap();
        }
        8 => {
            d.get_map("m")
                .insert(&format!("k{}", rng.gen_range(0..4)), i as i64)
                .unwrap();
        }
        9 => {
            let _ = peers[p].undo.undo();
        }
        10 => {
            let _ = peers[p].undo.redo();
        }
        11 | 12 => {
            let u = peers[p]
                .doc
                .export(ExportMode::updates(&peers[q].doc.oplog_vv()))
                .unwrap();
            let _ = peers[q].doc.import(&u);
        }
        13 => {
            let s = peers[p].doc.export(ExportMode::Snapshot).unwrap();
            let _ = peers[q].doc.import(&s);
        }
        14 => {
            let peer = peers[q].doc.peer_id();
            let shallow = rng.gen_bool(0.5);
            let bytes = if shallow {
                peers[q]
                    .doc
                    .export(ExportMode::shallow_snapshot(
                        &peers[q].doc.oplog_frontiers(),
                    ))
                    .unwrap()
            } else {
                peers[q].doc.export(ExportMode::Snapshot).unwrap()
            };
            let doc = LoroDoc::new();
            doc.import(&bytes).unwrap();
            doc.set_peer_id(peer).unwrap();
            let was_shallow = peers[q].shallow;
            peers[q] = new_peer(doc, shallow || was_shallow);
        }
        15 => {
            let r = (q + 1) % n;
            let a = peers[p]
                .doc
                .export(ExportMode::updates(&peers[q].doc.oplog_vv()))
                .unwrap();
            let b = peers[r]
                .doc
                .export(ExportMode::updates(&peers[q].doc.oplog_vv()))
                .unwrap();
            let _ = peers[q].doc.import_batch(&[b, a]);
        }
        _ => {}
    }
    peers[p].doc.commit();
}

fn mismatches(seed: u64, steps: usize) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let n = rng.gen_range(3..=5);
    let mut peers: Vec<Peer> = (0..n)
        .map(|i| {
            let d = LoroDoc::new();
            d.set_peer_id(i as u64 + 1).unwrap();
            let peer = new_peer(d, false);
            peer.doc.get_movable_list("l").push(-1).unwrap();
            peer.doc.get_text("t").insert(0, "s").unwrap();
            peer.doc.get_map("m").insert("init", -1).unwrap();
            peer.doc.commit();
            peer
        })
        .collect();
    for i in 0..steps {
        step(&mut rng, &mut peers, i);
    }
    let mut out = Vec::new();
    for (i, p) in peers.iter().enumerate() {
        p.doc.commit();
        if p.shallow || p.doc.is_shallow() {
            continue;
        }
        let replayed = replay(&p.doc);
        if replayed.get_deep_value() != p.doc.get_deep_value() {
            out.push(format!(
                "seed {seed} peer {i}: state={} replay={}",
                value(&p.doc),
                value(&replayed)
            ));
        }
    }
    out
}

#[test]
fn random_peers_agree_with_replay_of_their_history() {
    // Includes the seeds the issue reported for the root-map variant.
    let mut seeds: Vec<u64> = (0..120).collect();
    seeds.extend([426, 509, 591, 748]);
    let bad: Vec<String> = seeds
        .into_iter()
        .flat_map(|seed| mismatches(seed, 120))
        .collect();
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Which root of a name `get_value` and `get_deep_value` show depends on
/// whether each root is empty: an empty root does not hide a non-empty root of
/// another kind with the same name. A doc with decoded states answers that from
/// the states; a doc loaded from a snapshot, before anything decodes it, builds
/// the values. Both must choose the same roots.
#[test]
fn decoded_and_snapshot_loaded_docs_choose_the_same_roots() {
    let a = LoroDoc::new();
    a.set_peer_id(1).unwrap();
    a.config_text_style(loro::StyleConfigMap::default_rich_text_config());
    // A root of each kind that ends empty (a text keeps its mark's style
    // anchors), sharing its name with a non-empty map created before or after.
    let empty_root = |name: &str| match name.split('_').next().unwrap() {
        "tree" => {
            let tree = a.get_tree(name);
            let node = tree.create(None).unwrap();
            tree.delete(node).unwrap();
        }
        "list" => {
            a.get_list(name).push(1).unwrap();
            a.get_list(name).delete(0, 1).unwrap();
        }
        "movable" => {
            a.get_movable_list(name).push(1).unwrap();
            a.get_movable_list(name).delete(0, 1).unwrap();
        }
        "text" => {
            let text = a.get_text(name);
            text.insert(0, "abc").unwrap();
            text.mark(0..3, "bold", true).unwrap();
            text.delete(0, 3).unwrap();
        }
        _ => unreachable!(),
    };
    for kind in ["tree", "list", "movable", "text"] {
        let first = format!("{kind}_empty_first");
        empty_root(&first);
        a.get_map(first.as_str()).insert("k", 1).unwrap();
        let last = format!("{kind}_empty_last");
        a.get_map(last.as_str()).insert("k", 1).unwrap();
        empty_root(&last);
        // And the other way round: a non-empty root of this kind, an empty map.
        let full = format!("{kind}_full");
        match kind {
            "tree" => drop(a.get_tree(full.as_str()).create(None).unwrap()),
            "list" => a.get_list(full.as_str()).push(1).unwrap(),
            "movable" => a.get_movable_list(full.as_str()).push(1).unwrap(),
            _ => a.get_text(full.as_str()).insert(0, "y").unwrap(),
        }
        a.get_map(full.as_str()).insert("k", 1).unwrap();
        a.get_map(full.as_str()).delete("k").unwrap();
    }
    a.commit();

    let snapshot = a.export(ExportMode::Snapshot).unwrap();
    for hide in [false, true] {
        a.set_hide_empty_root_containers(hide);
        let loaded = LoroDoc::from_snapshot(&snapshot).unwrap();
        loaded.set_hide_empty_root_containers(hide);
        assert_eq!(
            a.get_value().to_json_value(),
            loaded.get_value().to_json_value(),
            "get_value, hide_empty_root_containers = {hide}"
        );
        let loaded = LoroDoc::from_snapshot(&snapshot).unwrap();
        loaded.set_hide_empty_root_containers(hide);
        assert_eq!(
            value(&a),
            value(&loaded),
            "get_deep_value, hide_empty_root_containers = {hide}"
        );
    }
}
