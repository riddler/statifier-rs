//! What more than one module's tests read.

// Every SCXML document the vendored corpus carries: the `source` of every
// case of the three suites, then every `.scxml` file under `cases/`.
pub(crate) fn corpus_documents() -> Vec<(String, Vec<u8>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("conformance/statifier");
    let mut documents = Vec::new();
    for suite in ["scion", "w3c", "statifier"] {
        let path = root.join("corpus").join(format!("{suite}.json"));
        let bytes = std::fs::read(&path).expect("a corpus file");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("corpus JSON");
        for case in json["cases"].as_array().expect("a case list") {
            let id = case["id"].as_str().expect("a case id");
            let source = case["source"].as_str().expect("a case source");
            documents.push((format!("{suite}/{id}"), source.as_bytes().to_vec()));
        }
    }
    let mut pending = vec![root.join("cases")];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("a cases directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "scxml")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    for path in files {
        let name = path
            .strip_prefix(&root)
            .expect("under the corpus")
            .display()
            .to_string();
        documents.push((name, std::fs::read(&path).expect("a case file")));
    }
    documents
}
