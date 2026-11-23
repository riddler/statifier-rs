//! Reading the vendored corpus: its manifest and every case of every suite
//! the manifest lists, in the manifest's order, each case's keys held to the
//! set the vendored `schema/case.json` names.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where the copy lands under the conformance directory: the vendoring
/// sequence's `DEST`, `conformance/statifier`.
pub const VENDORED_DIR: &str = "statifier";

/// The provenance record, beside the copy: the sequence's
/// `$DEST.vendored.json`.
pub const PROVENANCE_FILE: &str = "statifier.vendored.json";

/// This crate's registry, outside the copy.
pub const REGISTRY_FILE: &str = "registry.json";

/// The repository the copy is taken from, as the sequence records it.
pub const REFERENCE_REPO: &str = "https://github.com/riddler/statifier-ex";

/// This crate's name in its own registry.
pub const IMPLEMENTATION: &str = "statifier-rs";

/// The suites, in the order the corpus hash concatenates their files.
pub const SUITES: [&str; 3] = ["scion", "w3c", "statifier"];

/// This repository's conformance directory.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("conformance")
}

/// The JSON file at `relative` under `dir`, parsed.
pub fn read_json(dir: &Path, relative: &str) -> Result<Value, String> {
    let bytes = std::fs::read(dir.join(relative))
        .map_err(|error| format!("{relative} cannot be read: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{relative} is not JSON: {error}"))
}

/// The SHA-256 of `bytes`, written `sha256:` and lowercase hex.
pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut text = String::from("sha256:");
    for byte in digest.iter() {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// The corpus hash recomputed from the copy under `dir` (a conformance
/// directory), as the vendoring sequence computes it: the SHA-256 of the
/// bytes of `corpus/<suite>.json` concatenated in suite order (scion, w3c,
/// statifier), skipping a suite with no file. `None` when no corpus file
/// exists, which every caller treats as a failure.
pub fn corpus_hash(dir: &Path) -> Option<String> {
    let corpus = dir.join(VENDORED_DIR).join("corpus");
    let mut bytes = Vec::new();
    let mut found = false;
    for suite in SUITES {
        let path = corpus.join(format!("{suite}.json"));
        if path.is_file() {
            bytes.extend(std::fs::read(&path).ok()?);
            found = true;
        }
    }
    found.then(|| sha256(&bytes))
}

/// The claim a case counts toward: its suite, with the w3c suite split by
/// the case's `conformance` class; `None` for a w3c case with no class.
pub fn claim_of(item: &Value) -> Option<String> {
    let suite = item.get("suite").and_then(Value::as_str)?;
    if suite != "w3c" {
        return Some(suite.to_owned());
    }
    match item.get("conformance").and_then(Value::as_str) {
        Some(class @ ("mandatory" | "optional")) => Some(format!("w3c-{class}")),
        _ => None,
    }
}

/// The keys a case may carry, read out of the vendored `schema/case.json`
/// rather than written here: at every object the schema describes, the keys
/// its `properties` names, and below each the keys of the schema it names
/// for that key, through arrays by their `items` and through an object's
/// other members by an `additionalProperties` schema. A key the schema does
/// not name, at an object whose `additionalProperties` is `false`, is
/// refused rather than half-read, at any depth, so a corpus that grows a key
/// is noticed here the day it is vendored.
#[derive(Debug)]
pub struct CaseShape {
    schema: Value,
}

impl CaseShape {
    /// The shape `schema` (a parsed `schema/case.json`) gives a case.
    pub fn new(schema: Value) -> CaseShape {
        CaseShape { schema }
    }

    /// One sentence per key of `item` the schema does not name; empty when
    /// none.
    pub fn problems(&self, item: &Value) -> Vec<String> {
        let id = item
            .get("id")
            .map_or_else(|| "null".to_owned(), Value::to_string);
        let mut paths = Vec::new();
        unknown(item, Some(&self.schema), "", &mut paths);
        paths
            .into_iter()
            .map(|path| {
                format!("case {id} carries the key {path}, which schema/case.json does not name")
            })
            .collect()
    }
}

fn unknown(value: &Value, schema: Option<&Value>, path: &str, found: &mut Vec<String>) {
    let Some(schema) = schema else {
        return;
    };
    match value {
        Value::Object(object) => {
            let properties = schema.get("properties").and_then(Value::as_object);
            let others = schema.get("additionalProperties");
            for (key, member) in object {
                let at = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                match properties.and_then(|known| known.get(key)) {
                    Some(named) => unknown(member, Some(named), &at, found),
                    None => match others {
                        Some(Value::Bool(false)) => found.push(at),
                        Some(other @ Value::Object(_)) => unknown(member, Some(other), &at, found),
                        _ => {}
                    },
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                unknown(
                    item,
                    schema.get("items"),
                    &format!("{path}[{index}]"),
                    found,
                );
            }
        }
        _ => {}
    }
}

/// One suite of the corpus, as the manifest lists it, with its cases.
#[derive(Debug)]
pub struct Suite {
    /// The suite's name.
    pub suite: String,
    /// Every case the file holds, in its order.
    pub cases: Vec<Value>,
}

/// The vendored corpus: the manifest's corpus hash and every case of every
/// suite the manifest lists, in the manifest's order.
#[derive(Debug)]
pub struct Corpus {
    /// The manifest's `corpus_hash`.
    pub corpus_hash: String,
    /// The suites, in the manifest's order.
    pub suites: Vec<Suite>,
}

impl Corpus {
    /// Reads the corpus in the copy under `dir` (a conformance directory),
    /// refusing a manifest or suite file that does not read, a case with no
    /// id, an id two cases share, and a case carrying a key the case schema
    /// does not name.
    pub fn load(dir: &Path) -> Result<Corpus, String> {
        let copy = dir.join(VENDORED_DIR);
        let manifest = read_json(&copy, "manifest.json")?;
        let corpus_hash = manifest
            .get("corpus_hash")
            .and_then(Value::as_str)
            .ok_or("manifest.json has no corpus_hash")?
            .to_owned();
        let shape = CaseShape::new(read_json(&copy, "schema/case.json")?);
        let listed = manifest
            .get("suites")
            .and_then(Value::as_array)
            .ok_or("manifest.json has no suites array")?;
        let mut suites = Vec::new();
        for entry in listed {
            suites.push(read_suite(&copy, entry, &shape)?);
        }
        let corpus = Corpus {
            corpus_hash,
            suites,
        };
        let mut seen = BTreeMap::new();
        for item in corpus.cases() {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("a case has no id: {item}"))?;
            if seen.insert(id, ()).is_some() {
                return Err(format!("the corpus holds the case {id} more than once"));
            }
        }
        Ok(corpus)
    }

    /// Every case of every suite, in the manifest's suite order.
    pub fn cases(&self) -> impl Iterator<Item = &Value> {
        self.suites.iter().flat_map(|suite| suite.cases.iter())
    }

    /// Every case id, in corpus order.
    pub fn ids(&self) -> Vec<&str> {
        self.cases().filter_map(id_of).collect()
    }

    /// The cases of the named suite; none for a suite the corpus lacks.
    pub fn cases_of(&self, name: &str) -> &[Value] {
        self.suites
            .iter()
            .find(|suite| suite.suite == name)
            .map_or(&[], |suite| suite.cases.as_slice())
    }

    /// The id to case map.
    pub fn by_id(&self) -> BTreeMap<&str, &Value> {
        self.cases()
            .filter_map(|item| id_of(item).map(|id| (id, item)))
            .collect()
    }
}

/// A case's id, when it has one.
pub fn id_of(item: &Value) -> Option<&str> {
    item.get("id").and_then(Value::as_str)
}

fn read_suite(copy: &Path, entry: &Value, shape: &CaseShape) -> Result<Suite, String> {
    let (Some(suite), Some(file)) = (
        entry.get("suite").and_then(Value::as_str),
        entry.get("file").and_then(Value::as_str),
    ) else {
        return Err(format!(
            "manifest.json lists a suite without a suite and a file: {entry}"
        ));
    };
    let cases = read_json(copy, file)?
        .get("cases")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| format!("{file} holds no list of cases"))?;
    let problems: Vec<String> = cases.iter().flat_map(|item| shape.problems(item)).collect();
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    Ok(Suite {
        suite: suite.to_owned(),
        cases,
    })
}
