//! Pure boundary helpers for the general, top-level recorded view fixtures.
//!
//! Only generated Workbench captured targets may substitute database identity.
//! Expected fixtures are never normalized. No helper here constructs a demo/host,
//! reads the environment or filesystem, or performs clock/process/database work.

use std::collections::{BTreeMap, BTreeSet};

use radar_core::journal::work_map::{Snapshot, TagId};
use serde_json::{Value, json};
use uuid::Uuid;

pub const FIXTURE_SIZES: [(u16, u16); 2] = [(80, 24), (160, 40)];
pub type Surfaces = BTreeMap<String, Vec<((u16, u16), Value)>>;
pub type FixtureFiles = BTreeMap<String, Value>;

/// The ordinal follows FIXTURE_SIZES, never the lexical order of random UUIDs.
/// The two independent demo databases become ...0001 and ...0002, respectively.
#[derive(Clone, Default)]
pub struct IdentityMap {
    raw_to_canonical: BTreeMap<Uuid, Uuid>,
    canonical_to_raw: BTreeMap<Uuid, Uuid>,
}

impl IdentityMap {
    fn canonical(&mut self, raw: Uuid) -> Result<Uuid, String> {
        if let Some(canonical) = self.raw_to_canonical.get(&raw) {
            if self.canonical_to_raw.get(canonical) != Some(&raw) {
                return Err("database identity mapping is not a bijection".into());
            }
            return Ok(*canonical);
        }
        let ordinal = u64::try_from(self.raw_to_canonical.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or("too many fixture database identities")?;
        let canonical =
            Uuid::from_u128(0xf17e0000_0000_4000_8000_000000000000 | u128::from(ordinal));
        if self.canonical_to_raw.contains_key(&canonical) {
            return Err(
                "different database identities would collapse to one canonical identity".into(),
            );
        }
        self.raw_to_canonical.insert(raw, canonical);
        self.canonical_to_raw.insert(canonical, raw);
        Ok(canonical)
    }
}

#[derive(Clone, Copy)]
enum TargetKind {
    Group,
    Item,
    Source,
}

impl TargetKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Item => "item",
            Self::Source => "source",
        }
    }
}

type GroupTarget = (String, String, Snapshot, usize, u64);
type ItemOrSourceTarget = (String, String, Snapshot, usize, Option<TagId>, String);

enum CapturedTarget {
    Group(GroupTarget),
    ItemOrSource(ItemOrSourceTarget),
}

impl CapturedTarget {
    fn parse(text: &str, kind: TargetKind) -> Result<Self, String> {
        // Decode the ORIGINAL string through Snapshot's deny_unknown_fields
        // deserializer. Parsing to Value first would silently lose duplicate keys.
        let target = match kind {
            TargetKind::Group => serde_json::from_str::<GroupTarget>(text)
                .map(Self::Group)
                .map_err(|e| format!("invalid group target tuple: {e}"))?,
            TargetKind::Item | TargetKind::Source => {
                serde_json::from_str::<ItemOrSourceTarget>(text)
                    .map(Self::ItemOrSource)
                    .map_err(|e| format!("invalid item/source target tuple: {e}"))?
            }
        };
        let actual_kind = match &target {
            Self::Group(tuple) => &tuple.0,
            Self::ItemOrSource(tuple) => &tuple.0,
        };
        if actual_kind != kind.as_str() {
            return Err(format!(
                "expected {} target, found {actual_kind:?}",
                kind.as_str()
            ));
        }
        let snapshot = target.snapshot();
        if snapshot.event_sequence() < 0
            || snapshot.map_sequence() < 0
            || snapshot.gap_sequence() < 0
        {
            return Err("snapshot sequence fields must be nonnegative".into());
        }
        let raw = snapshot.database_id();
        let parsed =
            Uuid::parse_str(raw).map_err(|e| format!("invalid database UUID {raw:?}: {e}"))?;
        if parsed.hyphenated().to_string() != raw {
            return Err(format!(
                "database UUID is not canonical lowercase hyphenated text: {raw:?}"
            ));
        }
        Ok(target)
    }

    fn snapshot(&self) -> &Snapshot {
        match self {
            Self::Group(tuple) => &tuple.2,
            Self::ItemOrSource(tuple) => &tuple.2,
        }
    }

    fn encode_with_database_id(mut self, canonical: Uuid) -> Result<String, String> {
        let snapshot = match &mut self {
            Self::Group(tuple) => &mut tuple.2,
            Self::ItemOrSource(tuple) => &mut tuple.2,
        };
        let mut value = serde_json::to_value(&*snapshot).map_err(|e| e.to_string())?;
        value["database_id"] = Value::String(canonical.hyphenated().to_string());
        *snapshot = serde_json::from_value(value).map_err(|e| e.to_string())?;
        // Serialize the exact tuple types, retaining Snapshot's field order and
        // every other value. In particular, source identity remains an opaque
        // String: do not parse, reformat or normalize the JSON embedded in it.
        match self {
            Self::Group(tuple) => serde_json::to_string(&tuple),
            Self::ItemOrSource(tuple) => serde_json::to_string(&tuple),
        }
        .map_err(|e| e.to_string())
    }
}

fn captured_target_locations(view: &Value) -> Result<Vec<(String, TargetKind)>, String> {
    if view.get("scope").and_then(Value::as_str) != Some("work") {
        return Err("/scope: expected Workbench work view".into());
    }
    let panes = view
        .get("panes")
        .and_then(Value::as_array)
        .ok_or("/panes: expected array")?;
    // Each tuple describes one deliberately recognized target collection. An
    // extra or missing candidate fails instead of normalizing an arbitrary tree.
    let collections = [
        ("nav", "list", None, "/body/rows", 2, TargetKind::Group),
        ("middle", "list", None, "/body/rows", 5, TargetKind::Item),
        (
            "detail",
            "entries",
            None,
            "/body/rows",
            4,
            TargetKind::Source,
        ),
        (
            "detail",
            "extension",
            Some("work.map"),
            "/body/data/groups",
            1,
            TargetKind::Group,
        ),
        (
            "detail",
            "extension",
            Some("work.map"),
            "/body/data/items",
            5,
            TargetKind::Item,
        ),
    ];
    let mut locations = Vec::new();
    for (pane_id, node, capability, rows_path, count, kind) in collections {
        let matching: Vec<_> = panes
            .iter()
            .enumerate()
            .filter(|(_, pane)| {
                pane.get("pane").and_then(Value::as_str) == Some(pane_id)
                    && pane.pointer("/body/node").and_then(Value::as_str) == Some(node)
                    && capability.is_none_or(|capability| {
                        pane.pointer("/body/capability").and_then(Value::as_str) == Some(capability)
                    })
            })
            .collect();
        if matching.len() != 1 {
            return Err(format!(
                "/panes: expected one {pane_id}/{node}/{} collection, found {}",
                capability.unwrap_or("rows"),
                matching.len()
            ));
        }
        let (pane_index, pane) = matching[0];
        let path = format!("/panes/{pane_index}{rows_path}");
        if capability.is_some() && pane.pointer("/body/version") != Some(&json!(1)) {
            return Err(format!(
                "/panes/{pane_index}/body/version: expected work.map version 1"
            ));
        }
        let rows = pane
            .pointer(rows_path)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{path}: expected array"))?;
        if rows.len() != count {
            return Err(format!(
                "{path}: expected {count} captured targets, found {}",
                rows.len()
            ));
        }
        for (row_index, row) in rows.iter().enumerate() {
            let target_path = format!("{path}/{row_index}/target");
            if row.get("target").and_then(Value::as_str).is_none() {
                return Err(format!("{target_path}: expected captured target string"));
            }
            locations.push((target_path, kind));
        }
    }
    Ok(locations)
}

/// Canonicalize one generated size's Workbench view, atomically on success.
/// All 17 references must share one raw database ID; each invocation represents
/// a fresh size-demo database and must differ from all earlier invocations.
pub fn normalize_workbench_targets(
    view: &mut Value,
    identities: &mut IdentityMap,
) -> Result<(), String> {
    let locations = captured_target_locations(view)?;
    let mut raw_identity = None;
    let mut targets = Vec::new();
    for (path, kind) in locations {
        let text = view
            .pointer(&path)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{path}: expected captured target string"))?;
        let target = CapturedTarget::parse(text, kind).map_err(|e| format!("{path}: {e}"))?;
        let raw =
            Uuid::parse_str(target.snapshot().database_id()).map_err(|e| format!("{path}: {e}"))?;
        if raw_identity.is_some_and(|previous| previous != raw) {
            return Err(format!(
                "{path}: mixed database identities within one Workbench demo view"
            ));
        }
        raw_identity = Some(raw);
        targets.push((path, target));
    }
    let raw = raw_identity.ok_or("Workbench view has no captured database identity")?;
    if identities.raw_to_canonical.contains_key(&raw) {
        return Err("independent Workbench size demos reused one raw database identity".into());
    }
    let mut next_identities = identities.clone();
    let canonical = next_identities.canonical(raw)?;
    let replacements: Vec<_> = targets
        .into_iter()
        .map(|(path, target)| {
            target
                .encode_with_database_id(canonical)
                .map(|text| (path, text))
        })
        .collect::<Result<_, _>>()?;
    for (path, text) in replacements {
        // Locations were all resolved and validated above, before any writes.
        *view
            .pointer_mut(&path)
            .ok_or_else(|| format!("{path}: target disappeared"))? = Value::String(text);
    }
    *identities = next_identities;
    Ok(())
}

fn validate_surface_name(name: &str) -> Result<(), String> {
    let safe = name
        .split_once('_')
        .is_some_and(|(owner, surface)| !owner.is_empty() && !surface.is_empty())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if !safe {
        return Err(format!("invalid generated surface name {name:?}"));
    }
    Ok(())
}

/// Turn generated actual surfaces into the complete canonical file inventory.
/// The caller preserves coverage/size/round-trip assertions on its single sweep.
pub fn canonical_fixture_files(surfaces: &Surfaces) -> Result<FixtureFiles, String> {
    if surfaces.is_empty() {
        return Err("generated surface inventory is empty".into());
    }
    for (name, views) in surfaces {
        validate_surface_name(name)?;
        let sizes: BTreeSet<_> = views.iter().map(|(size, _)| *size).collect();
        if views.len() != FIXTURE_SIZES.len() || sizes != BTreeSet::from(FIXTURE_SIZES) {
            return Err(format!(
                "{name}: expected each fixture size exactly once, found {:?}",
                views.iter().map(|(size, _)| size).collect::<Vec<_>>()
            ));
        }
    }
    let mut identities = IdentityMap::default();
    let mut files = FixtureFiles::new();
    // A caller's Vec order cannot change canonical database identity assignment.
    for (w, h) in FIXTURE_SIZES {
        for (name, views) in surfaces {
            let mut view = views
                .iter()
                .find(|(size, _)| *size == (w, h))
                .unwrap()
                .1
                .clone();
            if name == "workbench_work" {
                normalize_workbench_targets(&mut view, &mut identities)
                    .map_err(|e| format!("{name}_{w}x{h}.json:{e}"))?;
            }
            files.insert(format!("{name}_{w}x{h}.json"), view);
        }
    }
    files.insert(
        "_surfaces.json".into(),
        json!({"surfaces": surfaces.keys().collect::<Vec<_>>() }),
    );
    Ok(files)
}

/// Only top-level generated JSON belongs to this contract. Handwritten fixtures
/// and all nested fixture sets (including reference/) have separate owners.
pub fn is_generated_fixture_path(path: &str) -> bool {
    !path.contains('/')
        && !path.contains('\\')
        && !path.starts_with("_hand_")
        && path.ends_with(".json")
}

fn generated_inventory<'a>(
    files: &'a FixtureFiles,
    label: &str,
) -> Result<BTreeSet<&'a str>, String> {
    let manifest = files
        .get("_surfaces.json")
        .ok_or_else(|| format!("{label}: missing _surfaces.json"))?;
    let manifest_object = manifest
        .as_object()
        .ok_or_else(|| format!("{label}:_surfaces.json: expected object"))?;
    if manifest_object.len() != 1 {
        return Err(format!(
            "{label}:_surfaces.json: expected only surfaces field"
        ));
    }
    let surfaces = manifest
        .get("surfaces")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label}:_surfaces.json:/surfaces: expected array"))?;
    if surfaces.is_empty() {
        return Err(format!("{label}:_surfaces.json:/surfaces: empty inventory"));
    }
    let mut declared = BTreeSet::from(["_surfaces.json".to_string()]);
    let mut previous: Option<&str> = None;
    for (index, surface) in surfaces.iter().enumerate() {
        let name = surface
            .as_str()
            .ok_or_else(|| format!("{label}:_surfaces.json:/surfaces/{index}: expected name"))?;
        validate_surface_name(name)
            .map_err(|e| format!("{label}:_surfaces.json:/surfaces/{index}: {e}"))?;
        if previous.is_some_and(|previous| previous >= name) {
            return Err(format!(
                "{label}:_surfaces.json:/surfaces/{index}: names must be sorted and unique"
            ));
        }
        previous = Some(name);
        for (w, h) in FIXTURE_SIZES {
            declared.insert(format!("{name}_{w}x{h}.json"));
        }
    }
    let present: BTreeSet<_> = files
        .keys()
        .filter(|path| is_generated_fixture_path(path))
        .map(String::as_str)
        .collect();
    let declared_refs: BTreeSet<_> = declared.iter().map(String::as_str).collect();
    if present != declared_refs {
        return Err(format!(
            "{label}: generated inventory differs from _surfaces.json; missing {:?}; extra {:?}",
            declared_refs.difference(&present).collect::<Vec<_>>(),
            present.difference(&declared_refs).collect::<Vec<_>>()
        ));
    }
    Ok(present)
}

fn pointer_component(text: &str) -> String {
    text.replace('~', "~0").replace('/', "~1")
}

fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    if expected == actual {
        return None;
    }
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let keys: BTreeSet<_> = expected.keys().chain(actual.keys()).collect();
            for key in keys {
                let path = format!("{path}/{}", pointer_component(key));
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => {
                        if let Some(difference) = first_difference(expected, actual, &path) {
                            return Some(difference);
                        }
                    }
                    (None, Some(_)) => return Some(format!("{path}: unexpected generated field")),
                    (Some(_), None) => return Some(format!("{path}: missing generated field")),
                    _ => unreachable!(),
                }
            }
        }
        (Value::Array(expected), Value::Array(actual)) => {
            if expected.len() != actual.len() {
                return Some(format!(
                    "{path}: expected {} entries, generated {}",
                    expected.len(),
                    actual.len()
                ));
            }
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                if let Some(difference) =
                    first_difference(expected, actual, &format!("{path}/{index}"))
                {
                    return Some(difference);
                }
            }
        }
        _ => return Some(format!("{path}: expected {expected}, generated {actual}")),
    }
    None
}

/// Compare expected files verbatim as JSON values: no identity substitution,
/// regeneration or repair is permitted on the expected side of this boundary.
pub fn compare_fixture_files(actual: &FixtureFiles, expected: &FixtureFiles) -> Result<(), String> {
    let actual_inventory = generated_inventory(actual, "generated")?;
    let expected_inventory = generated_inventory(expected, "committed")?;
    if actual_inventory != expected_inventory {
        return Err(format!(
            "generated/committed inventories differ; missing {:?}; extra {:?}",
            expected_inventory
                .difference(&actual_inventory)
                .collect::<Vec<_>>(),
            actual_inventory
                .difference(&expected_inventory)
                .collect::<Vec<_>>()
        ));
    }
    for name in actual_inventory {
        if let Some(difference) = first_difference(&expected[name], &actual[name], "") {
            return Err(format!("{name}:{difference}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW_A: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";
    const RAW_B: &str = "00000001-0000-4000-8000-000000000001";
    const RAW_C: &str = "11111111-1111-4111-8111-111111111111";
    const RAW_D: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
    const CANONICAL_A: &str = "f17e0000-0000-4000-8000-000000000001";
    const CANONICAL_B: &str = "f17e0000-0000-4000-8000-000000000002";
    const NAV: &str = "/panes/0/body/rows/0/target";
    const ITEM: &str = "/panes/1/body/rows/0/target";
    const SOURCE: &str = "/panes/2/body/rows/0/target";
    const MAP_GROUP: &str = "/panes/3/body/data/groups/0/target";
    const MAP_ITEM: &str = "/panes/3/body/data/items/0/target";

    fn snapshot(raw: &str) -> Value {
        json!({"database_id":raw, "event_sequence":7, "map_sequence":17, "gap_sequence":0})
    }

    fn target(kind: TargetKind, id: &str, raw: &str) -> Value {
        let tuple = match kind {
            TargetKind::Group => json!([kind.as_str(), id, snapshot(raw), 0, 8]),
            TargetKind::Item | TargetKind::Source => {
                json!([kind.as_str(), id, snapshot(raw), 0, null, ""])
            }
        };
        Value::String(serde_json::to_string(&tuple).unwrap())
    }

    fn work_view(raw: &str) -> Value {
        let items: Vec<_> = (1..=5)
            .map(|i| {
                json!({
                    "target": target(TargetKind::Item, &format!("watch-item-{i}"), raw),
                    "text": format!("Watch item {i}")
                })
            })
            .collect();
        let sources: Vec<_> = (1..=4).map(|i| json!({
            "target": target(TargetKind::Source, &format!(r#"["watch-source-{i}", {{"id": "WATCH-{i}"}}]"#), raw)
        })).collect();
        let group = json!({"target":target(TargetKind::Group,"watch-group",raw)});
        json!({
            "scope":"work", "version":1,
            "panes":[
                {"pane":"nav", "body":{"node":"list", "rows":[
                    {"target":target(TargetKind::Group,"all",raw)}, group.clone()
                ]}},
                {"pane":"middle", "body":{"node":"list", "rows":items.clone()}},
                {"pane":"detail", "body":{"node":"entries", "rows":sources}},
                {"pane":"detail", "body":{"node":"extension", "capability":"work.map", "version":1,
                    "data":{"groups":[group], "items":items, "target":"work-area:map"}}},
                {"pane":"detail", "body":{"node":"extension", "capability":"work.timeline", "version":1,
                    "data":{"target":"work-area:timeline"}}}
            ]
        })
    }

    fn surfaces(raw_a: &str, raw_b: &str) -> Surfaces {
        BTreeMap::from([
            (
                "slack_feed".into(),
                vec![
                    ((80, 24), json!({"clock":"07:11"})),
                    ((160, 40), json!({"clock":"07:11"})),
                ],
            ),
            (
                "workbench_work".into(),
                vec![((80, 24), work_view(raw_a)), ((160, 40), work_view(raw_b))],
            ),
        ])
    }

    fn fixture_files() -> FixtureFiles {
        canonical_fixture_files(&surfaces(RAW_A, RAW_B)).unwrap()
    }

    fn read_target(view: &Value, path: &str) -> Value {
        serde_json::from_str(view.pointer(path).unwrap().as_str().unwrap()).unwrap()
    }

    fn change_target(view: &mut Value, path: &str, change: impl FnOnce(&mut Value)) {
        let mut tuple = read_target(view, path);
        change(&mut tuple);
        *view.pointer_mut(path).unwrap() = Value::String(serde_json::to_string(&tuple).unwrap());
    }

    fn normalize(view: &mut Value) -> Result<(), String> {
        normalize_workbench_targets(view, &mut IdentityMap::default())
    }

    #[test]
    fn all_seventeen_references_keep_one_canonical_identity() {
        let mut view = work_view(RAW_A);
        let original = view.clone();
        normalize(&mut view).unwrap();
        let locations = captured_target_locations(&view).unwrap();
        assert_eq!(locations.len(), 17);
        let mut restored = view.clone();
        for (path, _) in locations {
            let mut expected = read_target(&original, &path);
            let original_database_id = expected[2]["database_id"].clone();
            expected[2]["database_id"] = json!(CANONICAL_A);
            assert_eq!(read_target(&view, &path), expected, "{path}");
            change_target(&mut restored, &path, |target| {
                target[2]["database_id"] = original_database_id;
            });
        }
        assert_eq!(restored, original, "only captured database IDs may change");
    }

    #[test]
    fn distinct_demo_identities_are_stable_in_size_encounter_order() {
        let files = fixture_files();
        let mut other = surfaces(RAW_C, RAW_D);
        other.get_mut("workbench_work").unwrap().reverse();
        let again = canonical_fixture_files(&other).unwrap();
        assert_eq!(
            files, again,
            "random UUID lexical order must not affect canonical IDs"
        );
        assert_eq!(
            read_target(&files["workbench_work_80x24.json"], NAV)[2]["database_id"],
            CANONICAL_A
        );
        assert_eq!(
            read_target(&files["workbench_work_160x40.json"], NAV)[2]["database_id"],
            CANONICAL_B
        );
        let mut identities = IdentityMap::default();
        let raw = Uuid::parse_str(RAW_A).unwrap();
        assert_eq!(
            identities.canonical(raw).unwrap(),
            identities.canonical(raw).unwrap()
        );
        let distinct = Uuid::parse_str(CANONICAL_A).unwrap();
        assert_ne!(
            identities.canonical(raw).unwrap(),
            identities.canonical(distinct).unwrap()
        );
        assert_eq!(
            identities.raw_to_canonical.len(),
            identities.canonical_to_raw.len()
        );
    }

    #[test]
    fn size_demos_cannot_reuse_one_raw_database_identity() {
        let error = canonical_fixture_files(&surfaces(RAW_A, RAW_A)).unwrap_err();
        assert!(
            error.contains("independent Workbench size demos"),
            "{error}"
        );
    }

    #[test]
    fn mixed_database_references_are_rejected_without_partial_changes() {
        for path in [NAV, ITEM, SOURCE, MAP_GROUP, MAP_ITEM] {
            let mut view = work_view(RAW_A);
            change_target(&mut view, path, |target| {
                target[2]["database_id"] = json!(RAW_B)
            });
            let original = view.clone();
            let mut identities = IdentityMap::default();
            let error = normalize_workbench_targets(&mut view, &mut identities).unwrap_err();
            assert!(
                error.contains("mixed database identities"),
                "{path}: {error}"
            );
            assert_eq!(view, original);
            assert!(identities.raw_to_canonical.is_empty());
        }
    }

    #[test]
    fn database_ids_must_be_canonical_uuids() {
        for raw in [
            "",
            "not-a-uuid",
            "FFFFFFFF-FFFF-4FFF-8FFF-FFFFFFFFFFFF",
            "ffffffffffff4fff8fffffffffffffff",
            "{ffffffff-ffff-4fff-8fff-ffffffffffff}",
        ] {
            let error = normalize(&mut work_view(raw)).unwrap_err();
            assert!(error.contains("UUID"), "{raw:?}: {error}");
        }
    }

    #[test]
    fn malformed_and_extra_tuple_fields_are_rejected() {
        for path in [NAV, ITEM, SOURCE, MAP_GROUP, MAP_ITEM] {
            for replacement in [
                json!(null),
                json!({}),
                json!("not JSON"),
                json!("[]"),
                json!("{}"),
            ] {
                let mut view = work_view(RAW_A);
                *view.pointer_mut(path).unwrap() = replacement;
                assert!(normalize(&mut view).is_err(), "{path}");
            }
            for extra in [false, true] {
                let mut view = work_view(RAW_A);
                change_target(&mut view, path, |target| {
                    if extra {
                        target.as_array_mut().unwrap().push(json!("extra"));
                    } else {
                        target.as_array_mut().unwrap().pop();
                    }
                });
                assert!(normalize(&mut view).is_err(), "{path}: extra={extra}");
            }
            let mut view = work_view(RAW_A);
            change_target(&mut view, path, |target| {
                target[0] = json!("unexpected-kind")
            });
            assert!(normalize(&mut view).unwrap_err().contains("expected"));
        }
    }

    #[test]
    fn captured_target_fields_reject_wrong_wire_types() {
        for path in [NAV, ITEM, SOURCE, MAP_GROUP, MAP_ITEM] {
            for (field, invalid) in [
                ("/0", json!(null)),
                ("/1", json!(42)),
                ("/2", json!([])),
                ("/3", json!(-1)),
                ("/3", json!("0")),
                ("/4", json!(false)),
            ] {
                let mut view = work_view(RAW_A);
                change_target(&mut view, path, |target| {
                    *target.pointer_mut(field).unwrap() = invalid;
                });
                assert!(normalize(&mut view).is_err(), "{path}: {field}");
            }
        }
        for path in [ITEM, SOURCE, MAP_ITEM] {
            let mut view = work_view(RAW_A);
            change_target(&mut view, path, |target| target[5] = json!(null));
            assert!(normalize(&mut view).is_err(), "{path}: text filter");
        }
        for path in [NAV, MAP_GROUP] {
            let mut view = work_view(RAW_A);
            change_target(&mut view, path, |target| target[4] = json!(-1));
            assert!(normalize(&mut view).is_err(), "{path}: group revision");
        }
    }

    #[test]
    fn snapshot_missing_unknown_and_duplicate_fields_are_rejected() {
        for field in [
            "database_id",
            "event_sequence",
            "map_sequence",
            "gap_sequence",
        ] {
            let mut view = work_view(RAW_A);
            change_target(&mut view, ITEM, |target| {
                target[2].as_object_mut().unwrap().remove(field);
            });
            assert!(normalize(&mut view).is_err(), "missing {field}");
            let mut view = work_view(RAW_A);
            change_target(&mut view, ITEM, |target| target[2][field] = json!(null));
            assert!(normalize(&mut view).is_err(), "null {field}");
        }
        let mut view = work_view(RAW_A);
        change_target(&mut view, ITEM, |target| target[2]["extra"] = json!(true));
        assert!(normalize(&mut view).unwrap_err().contains("unknown field"));
        for field in [
            "database_id",
            "event_sequence",
            "map_sequence",
            "gap_sequence",
        ] {
            let mut view = work_view(RAW_A);
            let text = view.pointer(ITEM).unwrap().as_str().unwrap();
            let value = snapshot(RAW_A)[field].to_string();
            let field_text = format!("\"{field}\":{value}");
            let duplicate = text.replace(&field_text, &format!("{field_text},{field_text}"));
            assert_ne!(duplicate, text);
            *view.pointer_mut(ITEM).unwrap() = json!(duplicate);
            assert!(
                normalize(&mut view)
                    .unwrap_err()
                    .contains("duplicate field")
            );
        }
    }

    #[test]
    fn negative_or_wrongly_typed_snapshot_sequences_are_rejected() {
        for field in ["event_sequence", "map_sequence", "gap_sequence"] {
            for invalid in [json!(-1), json!(1.5), json!("7"), json!(true)] {
                let mut view = work_view(RAW_A);
                change_target(&mut view, ITEM, |target| target[2][field] = invalid);
                assert!(normalize(&mut view).is_err(), "{field}");
            }
        }
    }

    #[test]
    fn missing_extra_or_unrecognized_target_collections_are_rejected() {
        for rows_path in [
            "/panes/0/body/rows",
            "/panes/1/body/rows",
            "/panes/2/body/rows",
            "/panes/3/body/data/groups",
            "/panes/3/body/data/items",
        ] {
            for extra in [false, true] {
                let mut view = work_view(RAW_A);
                let rows = view.pointer_mut(rows_path).unwrap().as_array_mut().unwrap();
                if extra {
                    rows.push(rows[0].clone());
                } else {
                    rows.pop();
                }
                assert!(normalize(&mut view).is_err(), "{rows_path}: extra={extra}");
            }
        }
        for path in [
            "/scope",
            "/panes/0/pane",
            "/panes/1/body/node",
            "/panes/2/body/node",
            "/panes/3/body/capability",
            "/panes/3/body/version",
        ] {
            let mut view = work_view(RAW_A);
            *view.pointer_mut(path).unwrap() = json!("unexpected");
            assert!(normalize(&mut view).is_err(), "{path}");
        }
        let mut view = work_view(RAW_A);
        let duplicate = view["panes"][0].clone();
        view["panes"].as_array_mut().unwrap().push(duplicate);
        assert!(normalize(&mut view).is_err());
    }

    #[test]
    fn unrelated_uuid_strings_and_opaque_source_identity_are_untouched() {
        let opaque =
            format!(r#"[ "watch-source", {{ "database_id" : "{RAW_A}", "id": "WATCH-12" }} ]"#);
        let mut view = work_view(RAW_A);
        view["unrelated_uuid"] = json!(RAW_A);
        view["unknown_location"] = json!({"target":target(TargetKind::Item,RAW_A,RAW_A)});
        view["panes"][4]["body"]["data"]["uuid"] = json!(RAW_A);
        change_target(&mut view, SOURCE, |target| target[1] = json!(opaque));
        let unknown = view["unknown_location"].clone();
        normalize(&mut view).unwrap();
        assert_eq!(view["unrelated_uuid"], RAW_A);
        assert_eq!(view["unknown_location"], unknown);
        assert_eq!(view["panes"][4]["body"]["data"]["uuid"], RAW_A);
        assert_eq!(read_target(&view, SOURCE)[1], opaque);
    }

    fn assert_semantic_change_visible(path: &str, field: &str, value: Value) {
        let expected = fixture_files();
        let mut actual = surfaces(RAW_A, RAW_B);
        let view = &mut actual.get_mut("workbench_work").unwrap()[0].1;
        change_target(view, path, |target| {
            *target.pointer_mut(field).unwrap() = value
        });
        let actual = canonical_fixture_files(&actual).unwrap();
        let error = compare_fixture_files(&actual, &expected).unwrap_err();
        assert!(
            error.contains("workbench_work_80x24.json") && error.contains(path),
            "{error}"
        );
    }

    #[test]
    fn item_target_nonidentity_semantics_remain_visible() {
        for path in [ITEM, MAP_ITEM] {
            for (field, value) in [
                ("/1", json!("watch-item-changed")),
                ("/3", json!(2)),
                ("/4", json!("watch-group-changed")),
                ("/5", json!("Watch filter changed")),
                ("/2/event_sequence", json!(8)),
                ("/2/map_sequence", json!(18)),
                ("/2/gap_sequence", json!(1)),
            ] {
                assert_semantic_change_visible(path, field, value);
            }
        }
    }

    #[test]
    fn group_target_nonidentity_semantics_remain_visible() {
        for path in [NAV, MAP_GROUP] {
            for (field, value) in [
                ("/1", json!("watch-group-changed")),
                ("/3", json!(2)),
                ("/4", json!(9)),
                ("/2/event_sequence", json!(8)),
                ("/2/map_sequence", json!(18)),
                ("/2/gap_sequence", json!(1)),
            ] {
                assert_semantic_change_visible(path, field, value);
            }
        }
    }

    #[test]
    fn source_target_nonidentity_semantics_remain_visible() {
        for (field, value) in [
            ("/1", json!(r#"["changed-source",{"id":"WATCH-99"}]"#)),
            ("/3", json!(2)),
            ("/4", json!("watch-group-changed")),
            ("/5", json!("Watch filter changed")),
            ("/2/event_sequence", json!(8)),
            ("/2/map_sequence", json!(18)),
            ("/2/gap_sequence", json!(1)),
        ] {
            assert_semantic_change_visible(SOURCE, field, value);
        }
    }

    #[test]
    fn corrupted_expected_database_identity_fails_direct_comparison() {
        let actual = fixture_files();
        let mut expected = actual.clone();
        change_target(
            expected.get_mut("workbench_work_80x24.json").unwrap(),
            ITEM,
            |target| {
                target[2]["database_id"] = json!(RAW_C);
            },
        );
        assert!(
            compare_fixture_files(&actual, &expected)
                .unwrap_err()
                .contains(ITEM)
        );
        // Even a consistent replacement of every committed reference must fail:
        // independently normalizing the expected side would hide this corruption.
        let mut expected = actual.clone();
        let view = expected.get_mut("workbench_work_80x24.json").unwrap();
        for (path, _) in captured_target_locations(view).unwrap() {
            change_target(view, &path, |target| {
                target[2]["database_id"] = json!(RAW_C)
            });
        }
        assert!(compare_fixture_files(&actual, &expected).is_err());
    }

    #[test]
    fn surface_inventory_requires_both_exact_sizes_and_safe_names() {
        for sizes in [
            vec![(80, 24)],
            vec![(80, 24), (80, 24)],
            vec![(80, 24), (160, 41)],
            vec![(80, 24), (160, 40), (40, 10)],
        ] {
            let surfaces = BTreeMap::from([(
                "slack_feed".into(),
                sizes.into_iter().map(|size| (size, json!({}))).collect(),
            )]);
            assert!(canonical_fixture_files(&surfaces).is_err());
        }
        assert!(canonical_fixture_files(&Surfaces::new()).is_err());
        for name in [
            "_hand_view",
            "reference/view",
            "../work",
            "view",
            "work_",
            "Work_view",
        ] {
            let surfaces = BTreeMap::from([(
                name.into(),
                FIXTURE_SIZES
                    .into_iter()
                    .map(|size| (size, json!({})))
                    .collect(),
            )]);
            assert!(canonical_fixture_files(&surfaces).is_err(), "{name}");
        }
    }

    #[test]
    fn inventory_rejects_missing_extra_wrong_size_and_changed_manifest_files() {
        let expected = fixture_files();
        let mut missing = expected.clone();
        missing.remove("slack_feed_80x24.json");
        assert!(
            compare_fixture_files(&missing, &expected)
                .unwrap_err()
                .contains("missing")
        );
        let mut extra = expected.clone();
        extra.insert("slack_other_80x24.json".into(), json!({}));
        assert!(
            compare_fixture_files(&extra, &expected)
                .unwrap_err()
                .contains("extra")
        );
        let mut wrong_size = expected.clone();
        let view = wrong_size.remove("slack_feed_80x24.json").unwrap();
        wrong_size.insert("slack_feed_80x25.json".into(), view);
        assert!(compare_fixture_files(&wrong_size, &expected).is_err());
        for manifest in [
            json!({"surfaces":["slack_feed"]}),
            json!({"surfaces":["workbench_work","slack_feed"]}),
            json!({"surfaces":["slack_feed","slack_feed","workbench_work"]}),
            json!({"surfaces":["slack_feed","workbench_work"],"extra":true}),
            json!({"surfaces":[]}),
        ] {
            let mut changed = expected.clone();
            changed.insert("_surfaces.json".into(), manifest);
            assert!(compare_fixture_files(&expected, &changed).is_err());
        }
        let mut fewer = expected.clone();
        fewer.remove("workbench_work_80x24.json");
        fewer.remove("workbench_work_160x40.json");
        fewer.insert("_surfaces.json".into(), json!({"surfaces":["slack_feed"]}));
        assert!(
            compare_fixture_files(&fewer, &expected)
                .unwrap_err()
                .contains("inventories differ")
        );
    }

    #[test]
    fn handwritten_and_nested_fixture_sets_stay_outside_generated_inventory() {
        let actual = fixture_files();
        let mut expected = actual.clone();
        for name in [
            "_hand_watch_80x24.json",
            "reference/workbench_work_80x24.json",
            "nested/_surfaces.json",
            "nested\\work_80x24.json",
            "README.md",
        ] {
            assert!(!is_generated_fixture_path(name), "{name}");
            expected.insert(name.into(), json!({"unrelated":"fixture"}));
        }
        compare_fixture_files(&actual, &expected).unwrap();
        assert!(is_generated_fixture_path("_surfaces.json"));
        assert!(is_generated_fixture_path("workbench_work_80x24.json"));
        assert!(is_generated_fixture_path("unexpected.json"));
    }

    #[test]
    fn comparison_reports_the_filename_and_escaped_json_path() {
        let expected = fixture_files();
        compare_fixture_files(&expected, &expected).unwrap();
        let mut actual = expected.clone();
        actual.get_mut("slack_feed_80x24.json").unwrap()["clock"] = json!("09:11");
        let error = compare_fixture_files(&actual, &expected).unwrap_err();
        assert!(error.contains("slack_feed_80x24.json:/clock"), "{error}");
        let mut expected = expected;
        expected.get_mut("slack_feed_80x24.json").unwrap()["a/b~c"] = json!({"value":1});
        let mut actual = expected.clone();
        actual.get_mut("slack_feed_80x24.json").unwrap()["a/b~c"]["value"] = json!(2);
        assert!(
            compare_fixture_files(&actual, &expected)
                .unwrap_err()
                .contains("/a~1b~0c/value")
        );
    }
}
