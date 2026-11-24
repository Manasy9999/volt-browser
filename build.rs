// Embarque l'icône de Volty dans volt.exe (Explorateur, barre des tâches) et la liste des langues.
fn main() {
    println!("cargo:rerun-if-changed=logos/volt.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new().set_icon("logos/volt.ico").compile().expect("icône");
    }
    // Une langue = un fichier src/lang/<code>.json : la liste est générée ici, rien d'autre à déclarer (voir src/lang.rs).
    println!("cargo:rerun-if-changed=src/lang");
    let mut codes: Vec<String> = std::fs::read_dir("src/lang").expect("src/lang").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).filter_map(|p| Some(p.file_stem()?.to_str()?.to_string())).collect();
    codes.sort();
    let list: String = codes.iter().map(|c| format!("(\"{c}\", include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/src/lang/{c}.json\"))),")).collect();
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("langs.rs"), format!("&[{list}]")).expect("langs.rs");
}
