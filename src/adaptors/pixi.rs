//! `pixi.toml` adaptor — per-key comment marking strategy.
//!
//! yard owns individual keys inside `pixi.toml`, each carrying a trailing
//! `# yard:managed default=<...>` marker. The user owns the surrounding
//! table headers, blank lines, comments, and any keys yard does not manage.
//!
//! v1 surface (per DESIGN.md):
//! - **Scalar**: `workspace.name` → `name = "<v>"  # yard:managed default="<v>"`.
//! - **Array**: `workspace.channels` → element-level reconciliation. User-added
//!   elements survive; removing one of yard's elements is a `Conflict`.
//! - **Map of scalars**: each entry under `[dependencies]` is its own managed
//!   key (`dependencies.<name>`); the `[dependencies]` *table* itself is the
//!   user's, so untracked deps are preserved verbatim.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, Formatted, Item, Table, Value};

use crate::adaptors::{Adaptor, ApplyOutcome, KeyAction, MergeError, PlanError};
use crate::{Contribution, RuntimeContext};

/// Fragment a module wants merged into `pixi.toml`.
///
/// Every field is optional/empty by default so a module only mentions what
/// it actually wants. Merging is per-field: scalars must agree across
/// modules (else `MergeError`), arrays union with dedup, maps merge with
/// per-key agreement.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PixiContribution {
    /// Scalar at `[workspace] name`.
    pub workspace_name: Option<String>,
    /// Array at `[workspace] channels`. Element-level reconciliation —
    /// user-added entries survive across applies.
    pub channels: Vec<String>,
    /// Map at `[dependencies]`. Keys are dep names, values are version
    /// constraints (e.g. `"3.11"`, `">=1.20"`, `"*"`).
    pub dependencies: BTreeMap<String, String>,
}

/// Merged intent the adaptor reconciles against the on-disk `pixi.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PixiDesired {
    pub workspace_name: Option<String>,
    pub channels: Vec<String>,
    pub dependencies: BTreeMap<String, String>,
}

/// Parse error surfaced when reconciling against a malformed `pixi.toml`.
#[derive(Debug, thiserror::Error)]
#[error("{path}: {kind}", path = .path.display())]
pub struct PixiParseError {
    pub path: PathBuf,
    pub kind: PixiParseErrorKind,
}

#[derive(Debug, thiserror::Error)]
pub enum PixiParseErrorKind {
    #[error("invalid TOML: {0}")]
    InvalidToml(String),
    #[error("`{key}` carries `yard:managed` without a `default=` payload")]
    ManagedMissingDefault { key: String },
    #[error(
        "`{key}`'s `default=` payload is a {default_shape} but the key holds a {value_shape}"
    )]
    DefaultShapeMismatch {
        key: String,
        default_shape: &'static str,
        value_shape: &'static str,
    },
}

impl PixiDesired {
    /// Collapse per-module contributions into a single merged desired.
    pub fn from_contributions<I>(contribs: I) -> Result<Self, MergeError>
    where
        I: IntoIterator<Item = (&'static str, PixiContribution)>,
    {
        let mut workspace_name: Option<(String, &'static str)> = None;
        let mut channels: Vec<String> = Vec::new();
        let mut dependencies: BTreeMap<String, (String, &'static str)> = BTreeMap::new();

        for (module, c) in contribs {
            if let Some(name) = c.workspace_name {
                match workspace_name.as_ref() {
                    Some((prev, prev_mod)) if prev != &name => {
                        return Err(MergeError {
                            adaptor: PixiAdaptor::ID,
                            key: "workspace.name".into(),
                            modules: vec![*prev_mod, module],
                            values: vec![prev.clone(), name],
                        });
                    }
                    Some(_) => {}
                    None => workspace_name = Some((name, module)),
                }
            }
            for ch in c.channels {
                if !channels.iter().any(|c| c == &ch) {
                    channels.push(ch);
                }
            }
            for (k, v) in c.dependencies {
                match dependencies.get(&k) {
                    Some((prev, prev_mod)) if prev != &v => {
                        return Err(MergeError {
                            adaptor: PixiAdaptor::ID,
                            key: format!("dependencies.{k}"),
                            modules: vec![*prev_mod, module],
                            values: vec![prev.clone(), v],
                        });
                    }
                    Some(_) => {}
                    None => {
                        dependencies.insert(k, (v, module));
                    }
                }
            }
        }

        Ok(Self {
            workspace_name: workspace_name.map(|(v, _)| v),
            channels,
            dependencies: dependencies
                .into_iter()
                .map(|(k, (v, _))| (k, v))
                .collect(),
        })
    }

    fn is_empty(&self) -> bool {
        self.workspace_name.is_none() && self.channels.is_empty() && self.dependencies.is_empty()
    }
}

pub struct PixiAdaptor;

impl PixiAdaptor {
    pub const ID: &'static str = "pixi";

    pub fn path(&self, ctx: &RuntimeContext) -> PathBuf {
        ctx.workspace.join("pixi.toml")
    }

    pub fn apply(
        &self,
        desired: &PixiDesired,
        existing: Option<&str>,
        ctx: &RuntimeContext,
    ) -> Result<ApplyOutcome, PixiParseError> {
        let path = self.path(ctx);
        let existing_text = existing.unwrap_or("");

        let mut doc: DocumentMut =
            existing_text
                .parse()
                .map_err(|e: toml_edit::TomlError| PixiParseError {
                    path: path.clone(),
                    kind: PixiParseErrorKind::InvalidToml(e.to_string()),
                })?;

        let (omit_paths, mut warnings) = scan_omits(existing_text);
        let omit_set: BTreeSet<&str> = omit_paths.iter().map(|s| s.as_str()).collect();
        let mut actions: Vec<KeyAction> = Vec::new();
        let mut matched_omits: BTreeSet<String> = BTreeSet::new();

        // ── workspace.name ─────────────────────────────────────────────
        {
            let key = "workspace.name";
            let marker = get_marker(&doc, &["workspace", "name"]);
            let wants = desired.workspace_name.as_deref();
            if wants.is_some() || marker.is_some() {
                if omit_set.contains(key) {
                    actions.push(KeyAction::Omitted { key: key.into() });
                    matched_omits.insert(key.into());
                } else {
                    process_scalar(
                        &mut doc, &mut actions, &["workspace", "name"], key, wants, marker,
                        &path,
                    )?;
                }
            }
        }

        // ── workspace.channels ─────────────────────────────────────────
        {
            let key = "workspace.channels";
            let marker = get_marker(&doc, &["workspace", "channels"]);
            let wants = if desired.channels.is_empty() {
                None
            } else {
                Some(desired.channels.as_slice())
            };
            if wants.is_some() || marker.is_some() {
                if omit_set.contains(key) {
                    actions.push(KeyAction::Omitted { key: key.into() });
                    matched_omits.insert(key.into());
                } else {
                    process_array(
                        &mut doc,
                        &mut actions,
                        &["workspace", "channels"],
                        key,
                        wants,
                        marker,
                        &path,
                    )?;
                }
            }
        }

        // ── dependencies.* ─────────────────────────────────────────────
        // Enumerate: desired ∪ on-disk-marked ∪ omits-for-deps.
        let mut dep_names: BTreeSet<String> = desired.dependencies.keys().cloned().collect();
        if let Some(deps_table) = doc.get("dependencies").and_then(|i| i.as_table()) {
            for (k, item) in deps_table.iter() {
                if let Some(v) = item.as_value() {
                    let suffix = v.decor().suffix().and_then(|r| r.as_str()).unwrap_or("");
                    if parse_marker(suffix).is_some() {
                        dep_names.insert(k.to_string());
                    }
                }
            }
        }
        for omit in &omit_paths {
            if let Some(rest) = omit.strip_prefix("dependencies.") {
                dep_names.insert(rest.to_string());
            }
        }
        for dep in &dep_names {
            let key = format!("dependencies.{dep}");
            let marker = get_marker(&doc, &["dependencies", dep.as_str()]);
            let wants = desired.dependencies.get(dep).map(|s| s.as_str());
            if omit_set.contains(key.as_str()) {
                actions.push(KeyAction::Omitted { key: key.clone() });
                matched_omits.insert(key);
                continue;
            }
            if wants.is_none() && marker.is_none() {
                // Omit-only entry with no managed/desired activity — handled
                // by stale-omit pass below.
                continue;
            }
            process_scalar(
                &mut doc,
                &mut actions,
                &["dependencies", dep.as_str()],
                &key,
                wants,
                marker,
                &path,
            )?;
        }

        // Stale-omit pass. Any valid-charset omit that didn't match a
        // currently-managed or currently-desired key still reports
        // `Omitted` (so the action log accounts for the user's marker) and
        // emits a warning so they know it's redundant.
        for omit in &omit_paths {
            if matched_omits.contains(omit) {
                continue;
            }
            actions.push(KeyAction::Omitted { key: omit.clone() });
            warnings.push(format!(
                "yard:omit {omit} is stale: yard no longer manages this key"
            ));
        }

        Ok(ApplyOutcome {
            contents: Some(doc.to_string()),
            actions,
            warnings,
        })
    }
}

impl Adaptor for PixiAdaptor {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn path(&self, ctx: &RuntimeContext) -> PathBuf {
        PixiAdaptor::path(self, ctx)
    }

    fn plan(
        &self,
        contribs: Vec<(&'static str, Contribution)>,
        existing: Option<&str>,
        ctx: &RuntimeContext,
    ) -> Result<ApplyOutcome, PlanError> {
        let mine = contribs.into_iter().map(|(module, c)| match c {
            Contribution::Pixi(p) => (module, p),
            other => unreachable!(
                "engine routed {} contribution to pixi adaptor",
                other.adaptor_id()
            ),
        });
        let desired = PixiDesired::from_contributions(mine)?;
        // Nothing to manage and no file on disk: signal "no file" so the
        // engine doesn't materialise an empty `pixi.toml`.
        if desired.is_empty() && existing.is_none() {
            return Ok(ApplyOutcome {
                contents: None,
                actions: Vec::new(),
                warnings: Vec::new(),
            });
        }
        self.apply(&desired, existing, ctx).map_err(|e| PlanError::Parse {
            adaptor: Self::ID,
            source: Box::new(e),
        })
    }
}

// ─── Marker parsing ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Marker {
    Managed { default_raw: String },
    ManagedMissingDefault,
    Overridden,
}

fn parse_marker(suffix: &str) -> Option<Marker> {
    // Suffixes from toml_edit look like `  # yard:managed default="x"\n`
    // or `  # other comment\n` or just whitespace. Find the `#` and parse
    // from there.
    let hash = suffix.find('#')?;
    let body = suffix[hash + 1..].trim_start();
    // Stop at a newline so a trailing newline in the suffix doesn't get
    // swallowed into the marker payload.
    let body = body.split('\n').next().unwrap_or(body).trim_end();

    if let Some(rest) = body.strip_prefix("yard:overridden") {
        // Trailing junk after `yard:overridden` is tolerated (and ignored).
        let _ = rest;
        return Some(Marker::Overridden);
    }
    if let Some(rest) = body.strip_prefix("yard:managed") {
        let rest = rest.trim_start();
        if rest.is_empty() {
            return Some(Marker::ManagedMissingDefault);
        }
        if let Some(default) = rest.strip_prefix("default=") {
            return Some(Marker::Managed {
                default_raw: default.trim().to_string(),
            });
        }
        return Some(Marker::ManagedMissingDefault);
    }
    None
}

fn get_marker(doc: &DocumentMut, path: &[&str]) -> Option<Marker> {
    let item = walk(doc, path)?;
    let v = item.as_value()?;
    let suffix = v.decor().suffix().and_then(|r| r.as_str()).unwrap_or("");
    parse_marker(suffix)
}

// ─── Doc traversal ────────────────────────────────────────────────────

fn walk<'a>(doc: &'a DocumentMut, path: &[&str]) -> Option<&'a Item> {
    let (last, init) = path.split_last()?;
    let mut table = doc.as_table();
    for seg in init {
        table = table.get(seg)?.as_table()?;
    }
    table.get(last)
}

fn walk_mut<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Option<&'a mut Item> {
    let (last, init) = path.split_last()?;
    let mut table: &mut Table = doc.as_table_mut();
    for seg in init {
        table = table.get_mut(seg)?.as_table_mut()?;
    }
    table.get_mut(last)
}

fn ensure_table_mut<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Option<&'a mut Table> {
    let mut table: &mut Table = doc.as_table_mut();
    for seg in path {
        let entry = table
            .entry(seg)
            .or_insert_with(|| Item::Table(Table::new()));
        let t = entry.as_table_mut()?;
        t.set_implicit(false);
        table = t;
    }
    Some(table)
}

fn remove_at(doc: &mut DocumentMut, path: &[&str]) {
    let Some((last, init)) = path.split_last() else {
        return;
    };
    let mut table: &mut Table = doc.as_table_mut();
    for seg in init {
        match table.get_mut(seg).and_then(|i| i.as_table_mut()) {
            Some(t) => table = t,
            None => return,
        }
    }
    table.remove(last);
}

// ─── Scalar reconciliation ────────────────────────────────────────────

fn process_scalar(
    doc: &mut DocumentMut,
    actions: &mut Vec<KeyAction>,
    path: &[&str],
    key: &str,
    wants: Option<&str>,
    marker: Option<Marker>,
    file_path: &Path,
) -> Result<(), PixiParseError> {
    let on_disk: Option<String> = walk(doc, path)
        .and_then(|i| i.as_value())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match marker {
        Some(Marker::Overridden) => {
            actions.push(KeyAction::Overridden { key: key.into() });
            return Ok(());
        }
        Some(Marker::ManagedMissingDefault) => {
            return Err(PixiParseError {
                path: file_path.to_path_buf(),
                kind: PixiParseErrorKind::ManagedMissingDefault { key: key.into() },
            });
        }
        _ => {}
    }

    let on_disk_managed_default: Option<Result<Value, ()>> = match &marker {
        Some(Marker::Managed { default_raw }) => Some(default_raw.parse::<Value>().map_err(|_| ())),
        _ => None,
    };

    match (wants, marker, on_disk) {
        // ── managed-marked, value on disk ─────────────────────────────
        (Some(want), Some(Marker::Managed { .. }), Some(on_disk_value)) => {
            match on_disk_managed_default {
                Some(Ok(Value::String(s))) => {
                    let default_str = s.value();
                    if default_str.as_str() == on_disk_value.as_str() {
                        if default_str == want {
                            actions.push(KeyAction::InSync { key: key.into() });
                        } else {
                            write_scalar(doc, path, want);
                            actions.push(KeyAction::Updated {
                                key: key.into(),
                                from: on_disk_value,
                                to: want.into(),
                            });
                        }
                    } else {
                        actions.push(KeyAction::Conflict {
                            key: key.into(),
                            on_disk: on_disk_value,
                            default: default_str.clone(),
                        });
                    }
                }
                Some(Ok(_)) => {
                    return Err(PixiParseError {
                        path: file_path.to_path_buf(),
                        kind: PixiParseErrorKind::DefaultShapeMismatch {
                            key: key.into(),
                            default_shape: "array",
                            value_shape: "string",
                        },
                    });
                }
                Some(Err(())) => {
                    // Unparsable default: treat as unmarked → Reemitted.
                    write_scalar(doc, path, want);
                    actions.push(KeyAction::Reemitted {
                        key: key.into(),
                        to: want.into(),
                    });
                }
                None => unreachable!("filtered above"),
            }
        }

        // ── unmarked value on disk, adaptor wants ─────────────────────
        (Some(want), None, Some(_)) => {
            write_scalar(doc, path, want);
            actions.push(KeyAction::Reemitted {
                key: key.into(),
                to: want.into(),
            });
        }

        // ── nothing on disk, adaptor wants ────────────────────────────
        (Some(want), None, None) => {
            write_scalar(doc, path, want);
            actions.push(KeyAction::Reemitted {
                key: key.into(),
                to: want.into(),
            });
        }

        // ── managed on disk, adaptor doesn't want (removal) ───────────
        (None, Some(Marker::Managed { .. }), Some(on_disk_value)) => {
            match on_disk_managed_default {
                Some(Ok(Value::String(s))) => {
                    let default_str = s.value();
                    if default_str.as_str() == on_disk_value.as_str() {
                        remove_at(doc, path);
                        actions.push(KeyAction::Deleted {
                            key: key.into(),
                            was: on_disk_value,
                        });
                    } else {
                        actions.push(KeyAction::Conflict {
                            key: key.into(),
                            on_disk: on_disk_value,
                            default: default_str.clone(),
                        });
                    }
                }
                Some(Ok(_)) => {
                    return Err(PixiParseError {
                        path: file_path.to_path_buf(),
                        kind: PixiParseErrorKind::DefaultShapeMismatch {
                            key: key.into(),
                            default_shape: "array",
                            value_shape: "string",
                        },
                    });
                }
                Some(Err(())) | None => {
                    // Unparsable default during removal — best we can do
                    // is leave the key alone. No fixture exercises this.
                }
            }
        }

        // Other combinations (no value on disk yet managed marker, etc.)
        // are structurally impossible — markers live on values.
        _ => {}
    }

    Ok(())
}

// ─── Array reconciliation ─────────────────────────────────────────────

fn process_array(
    doc: &mut DocumentMut,
    actions: &mut Vec<KeyAction>,
    path: &[&str],
    key: &str,
    wants: Option<&[String]>,
    marker: Option<Marker>,
    file_path: &Path,
) -> Result<(), PixiParseError> {
    let on_disk: Option<Vec<String>> = walk(doc, path)
        .and_then(|i| i.as_value())
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

    match marker {
        Some(Marker::Overridden) => {
            actions.push(KeyAction::Overridden { key: key.into() });
            return Ok(());
        }
        Some(Marker::ManagedMissingDefault) => {
            return Err(PixiParseError {
                path: file_path.to_path_buf(),
                kind: PixiParseErrorKind::ManagedMissingDefault { key: key.into() },
            });
        }
        _ => {}
    }

    let parsed_default: Option<Result<Vec<String>, ParseDefaultErr>> = match &marker {
        Some(Marker::Managed { default_raw }) => Some(match default_raw.parse::<Value>() {
            Ok(Value::Array(arr)) => Ok(arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()),
            Ok(_) => Err(ParseDefaultErr::ShapeMismatch),
            Err(_) => Err(ParseDefaultErr::Unparsable),
        }),
        _ => None,
    };

    match (wants, marker, on_disk) {
        (Some(want), Some(Marker::Managed { .. }), Some(on_disk_arr)) => {
            match parsed_default {
                Some(Ok(default_vec)) => {
                    let removed: bool = default_vec.iter().any(|d| !on_disk_arr.contains(d));
                    if removed {
                        actions.push(KeyAction::Conflict {
                            key: key.into(),
                            on_disk: serialize_default_array(&on_disk_arr),
                            default: serialize_default_array(&default_vec),
                        });
                    } else {
                        let user_additions: Vec<String> = on_disk_arr
                            .iter()
                            .filter(|x| !default_vec.contains(x))
                            .cloned()
                            .collect();
                        let new_value: Vec<String> =
                            want.iter().cloned().chain(user_additions).collect();
                        if new_value == on_disk_arr && default_vec.as_slice() == want {
                            actions.push(KeyAction::InSync { key: key.into() });
                        } else {
                            let from = serialize_default_array(&on_disk_arr);
                            let to = serialize_default_array(&new_value);
                            write_array(doc, path, &new_value, want);
                            actions.push(KeyAction::Updated {
                                key: key.into(),
                                from,
                                to,
                            });
                        }
                    }
                }
                Some(Err(ParseDefaultErr::ShapeMismatch)) => {
                    return Err(PixiParseError {
                        path: file_path.to_path_buf(),
                        kind: PixiParseErrorKind::DefaultShapeMismatch {
                            key: key.into(),
                            default_shape: "string",
                            value_shape: "array",
                        },
                    });
                }
                Some(Err(ParseDefaultErr::Unparsable)) => {
                    write_array(doc, path, want, want);
                    actions.push(KeyAction::Reemitted {
                        key: key.into(),
                        to: serialize_default_array(want),
                    });
                }
                None => unreachable!("filtered above"),
            }
        }

        (Some(want), None, Some(_)) | (Some(want), None, None) => {
            write_array(doc, path, want, want);
            actions.push(KeyAction::Reemitted {
                key: key.into(),
                to: serialize_default_array(want),
            });
        }

        (None, Some(Marker::Managed { .. }), Some(on_disk_arr)) => match parsed_default {
            Some(Ok(default_vec)) => {
                let removed: bool = default_vec.iter().any(|d| !on_disk_arr.contains(d));
                if removed {
                    actions.push(KeyAction::Conflict {
                        key: key.into(),
                        on_disk: serialize_default_array(&on_disk_arr),
                        default: serialize_default_array(&default_vec),
                    });
                } else {
                    remove_at(doc, path);
                    actions.push(KeyAction::Deleted {
                        key: key.into(),
                        was: serialize_default_array(&on_disk_arr),
                    });
                }
            }
            Some(Err(ParseDefaultErr::ShapeMismatch)) => {
                return Err(PixiParseError {
                    path: file_path.to_path_buf(),
                    kind: PixiParseErrorKind::DefaultShapeMismatch {
                        key: key.into(),
                        default_shape: "string",
                        value_shape: "array",
                    },
                });
            }
            Some(Err(ParseDefaultErr::Unparsable)) | None => {}
        },

        _ => {}
    }

    Ok(())
}

enum ParseDefaultErr {
    ShapeMismatch,
    Unparsable,
}

// ─── Writers ──────────────────────────────────────────────────────────

fn write_scalar(doc: &mut DocumentMut, path: &[&str], new_val: &str) {
    let new_suffix = format!("  # yard:managed default={}", serialize_default_string(new_val));
    if let Some(existing) = walk_mut(doc, path).and_then(|i| i.as_value_mut()) {
        // Preserve the prefix (typically `" "`) and any trailing newline
        // that lived in the original suffix, replacing only the comment.
        let prefix = existing.decor().prefix().cloned();
        let trailing = trailing_after_comment(
            existing
                .decor()
                .suffix()
                .and_then(|r| r.as_str())
                .unwrap_or(""),
        );
        *existing = Value::String(Formatted::new(new_val.to_string()));
        if let Some(p) = prefix {
            existing.decor_mut().set_prefix(p);
        } else {
            existing.decor_mut().set_prefix(" ");
        }
        existing
            .decor_mut()
            .set_suffix(format!("{new_suffix}{trailing}"));
        return;
    }
    // Fresh insert. Parent table is created on demand.
    let (last, init) = match path.split_last() {
        Some(x) => x,
        None => return,
    };
    let Some(table) = ensure_table_mut(doc, init) else {
        return;
    };
    let mut value = Value::String(Formatted::new(new_val.to_string()));
    value.decor_mut().set_prefix(" ");
    value.decor_mut().set_suffix(new_suffix);
    table.insert(last, Item::Value(value));
}

fn write_array(doc: &mut DocumentMut, path: &[&str], elements: &[String], default: &[String]) {
    let new_suffix = format!("  # yard:managed default={}", serialize_default_array(default));
    let mut arr = Array::new();
    for s in elements {
        arr.push(s.as_str());
    }
    if let Some(existing) = walk_mut(doc, path).and_then(|i| i.as_value_mut()) {
        let prefix = existing.decor().prefix().cloned();
        let trailing = trailing_after_comment(
            existing
                .decor()
                .suffix()
                .and_then(|r| r.as_str())
                .unwrap_or(""),
        );
        *existing = Value::Array(arr);
        if let Some(p) = prefix {
            existing.decor_mut().set_prefix(p);
        } else {
            existing.decor_mut().set_prefix(" ");
        }
        existing
            .decor_mut()
            .set_suffix(format!("{new_suffix}{trailing}"));
        return;
    }
    let (last, init) = match path.split_last() {
        Some(x) => x,
        None => return,
    };
    let Some(table) = ensure_table_mut(doc, init) else {
        return;
    };
    let mut value = Value::Array(arr);
    value.decor_mut().set_prefix(" ");
    value.decor_mut().set_suffix(new_suffix);
    table.insert(last, Item::Value(value));
}

/// Strip the existing comment from a suffix string, returning whatever
/// trailing whitespace (typically a newline) came after it. Used by
/// `write_*` so updating an in-place value preserves the line break that
/// followed the original `#`-comment.
fn trailing_after_comment(suffix: &str) -> String {
    if let Some(hash) = suffix.find('#') {
        if let Some(nl) = suffix[hash..].find('\n') {
            return suffix[hash + nl..].to_string();
        }
        // No newline after the comment — nothing to preserve.
        return String::new();
    }
    // No comment in the suffix at all: keep whatever whitespace was there
    // (could be `\n` for a value that previously had no marker).
    suffix.to_string()
}

// ─── Default-value serialization ──────────────────────────────────────

fn serialize_default_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn serialize_default_array(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&serialize_default_string(item));
    }
    out.push(']');
    out
}

// ─── Omit scanning ────────────────────────────────────────────────────

fn scan_omits(text: &str) -> (Vec<String>, Vec<String>) {
    let mut paths: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("# yard:omit") else {
            continue;
        };
        let arg = rest.trim();
        if arg.is_empty() {
            warnings.push("yard:omit with empty argument; ignoring".into());
            continue;
        }
        if is_valid_omit_path(arg) {
            if !paths.iter().any(|p| p == arg) {
                paths.push(arg.to_string());
            }
        } else {
            warnings.push(format!(
                "yard:omit `{arg}` is not a valid key path (allowed: [A-Za-z0-9_.-]+); ignoring"
            ));
        }
    }
    (paths, warnings)
}

fn is_valid_omit_path(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

#[cfg(test)]
mod tests;
